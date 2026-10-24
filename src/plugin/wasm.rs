//! Sandboxed Wasm validation and stateless analyze execution.
//!
//! # Sandbox
//!
//! Modules are parsed and executed by the `wasmi` interpreter only. There is
//! no JIT, native code, WASI, or host ABI surface. A valid module has **zero**
//! imports, no start function, 32-bit linear memory, and no reference types,
//! SIMD, tail calls, custom page sizes, or other optional proposals beyond
//! core Wasm plus bulk memory and sign extension.
//!
//! Limits are enforced at several layers:
//!
//! - module bytes are capped before parsing;
//! - `wasmi` enforced limits reject oversized modules;
//! - declared memory must fit the configured byte limit;
//! - the store's resource limiter caps growth;
//! - fuel bounds execution time without needing a clock;
//! - JSON input and output sizes are bounded;
//! - the recursion depth is bounded.
//!
//! Each call creates a fresh `Store` and instance, so plugin memory and globals
//! do not persist between analyze calls.
//!
//! # ABI v1
//!
//! A functionality module exports:
//!
//! ```text
//! (memory (export "memory") ...)                     ;; 32-bit linear memory
//! (func (export "amph_abi_version") (result i32))    ;; must return 1
//! (func (export "amph_alloc") (param i32) (result i32))
//! (func (export "amph_analyze") (param i32 i32) (result i64))
//! ```
//!
//! The host allocates `input.len()` bytes, writes the JSON request at the
//! returned pointer, and calls `amph_analyze(ptr, len)`. The result packs the
//! output pointer in the high 32 bits and the output length in the low 32 bits:
//!
//! ```text
//! packed = (output_ptr as u64) << 32 | output_len as u64
//! ```
//!
//! Output bytes must contain one JSON value. The host copies them out of the
//! instance before dropping it. A module may keep working buffers inside its
//! linear memory, but no state survives across host calls.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use wasmi::{
    Config, EnforcedLimits, Engine, ExternType, FuncType, Linker, Module, Store, StoreLimits,
    StoreLimitsBuilder, TypedFunc, ValType,
};

/// Supported plugin ABI version.
pub const ABI_VERSION: u32 = 1;

/// Maximum accepted Wasm module size (2 MiB).
pub const MAX_MODULE_BYTES: usize = 2 * 1024 * 1024;

/// Wasm linear-memory page size.
pub const WASM_PAGE_BYTES: usize = 64 * 1024;

/// Default per-call execution fuel.
pub const DEFAULT_FUEL: u64 = 20_000_000;

/// Resource limits for validation and execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLimits {
    /// Maximum module byte length.
    pub max_module_bytes: usize,
    /// Maximum linear memory size in bytes.
    pub max_memory_bytes: usize,
    /// Maximum JSON request size in bytes.
    pub max_input_bytes: usize,
    /// Maximum JSON response size in bytes.
    pub max_output_bytes: usize,
    /// Fuel available for one `amph_analyze` call.
    pub fuel: u64,
    /// Maximum Wasm call/control recursion depth.
    pub max_recursion_depth: usize,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_module_bytes: MAX_MODULE_BYTES,
            max_memory_bytes: 16 * 1024 * 1024,
            max_input_bytes: 1024 * 1024,
            max_output_bytes: 1024 * 1024,
            fuel: DEFAULT_FUEL,
            max_recursion_depth: 256,
        }
    }
}

/// Metadata for a validated module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleInfo {
    /// Value the module is expected to return from `amph_abi_version`.
    pub abi_version: u32,
    /// Declared initial memory pages.
    pub memory_pages: u64,
    /// Declared maximum memory pages, if any.
    pub memory_max_pages: Option<u64>,
}

/// Validate a module and return its ABI metadata.
pub fn validate_module(bytes: &[u8]) -> Result<ModuleInfo> {
    validate_module_with(bytes, &RuntimeLimits::default())
}

/// Validate a module against explicit limits.
pub fn validate_module_with(bytes: &[u8], limits: &RuntimeLimits) -> Result<ModuleInfo> {
    let (_, _, info) = compile(bytes, limits)?;
    Ok(info)
}

/// Execute a validated module with a JSON request and return JSON bytes.
pub fn analyze(module_bytes: &[u8], input: &[u8]) -> Result<Vec<u8>> {
    analyze_with(module_bytes, input, &RuntimeLimits::default())
}

/// Execute a module with an explicit limit set.
pub fn analyze_with(module_bytes: &[u8], input: &[u8], limits: &RuntimeLimits) -> Result<Vec<u8>> {
    let (engine, module, _) = compile(module_bytes, limits)?;
    if input.len() > limits.max_input_bytes {
        bail!(
            "plugin input is too large ({} bytes, limit {})",
            input.len(),
            limits.max_input_bytes
        );
    }
    let input_len =
        i32::try_from(input.len()).context("plugin input does not fit the Wasm ABI length")?;

    let store_limits = StoreLimitsBuilder::new()
        .memory_size(limits.max_memory_bytes)
        .table_elements(1024)
        .memories(1)
        .tables(1)
        .instances(2)
        .build();
    let mut store: Store<StoreLimits> = Store::new(&engine, store_limits);
    store.limiter(|store_limits| store_limits);
    store
        .set_fuel(limits.fuel)
        .context("cannot enable fuel metering for plugin execution")?;

    let linker = Linker::new(&engine);
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .context("cannot instantiate the plugin module")?;

    let abi: TypedFunc<(), i32> = instance
        .get_typed_func(&store, "amph_abi_version")
        .context("plugin is missing the ABI version export")?;
    let version = abi.call(&mut store, ()).context("plugin ABI call failed")?;
    if version as u32 != ABI_VERSION {
        bail!("plugin reported ABI {version}, but this build supports ABI {ABI_VERSION}");
    }

    let alloc: TypedFunc<i32, i32> = instance
        .get_typed_func(&store, "amph_alloc")
        .context("plugin is missing `amph_alloc`")?;
    let analyze: TypedFunc<(i32, i32), i64> = instance
        .get_typed_func(&store, "amph_analyze")
        .context("plugin is missing `amph_analyze`")?;
    let memory = instance
        .get_memory(&store, "memory")
        .ok_or_else(|| anyhow::anyhow!("plugin does not export `memory`"))?;

    let pointer = alloc
        .call(&mut store, input_len)
        .context("plugin allocation failed")?;
    if pointer < 0 {
        bail!("plugin returned a negative input pointer");
    }
    let pointer = pointer as usize;
    let input_end = pointer
        .checked_add(input.len())
        .ok_or_else(|| anyhow::anyhow!("plugin input pointer overflowed"))?;
    if input_end > memory.data_size(&store) {
        bail!(
            "plugin allocated {} bytes at {}, outside its {} byte memory",
            input.len(),
            pointer,
            memory.data_size(&store)
        );
    }
    memory.data_mut(&mut store)[pointer..input_end].copy_from_slice(input);

    let packed = analyze
        .call(&mut store, (pointer as i32, input_len))
        .map_err(|error| anyhow::anyhow!("plugin analyze call failed: {error}"))?
        as u64;
    let output_pointer = ((packed >> 32) & u64::from(u32::MAX)) as usize;
    let output_len = (packed & u64::from(u32::MAX)) as usize;
    if output_len > limits.max_output_bytes {
        bail!(
            "plugin output is too large ({output_len} bytes, limit {})",
            limits.max_output_bytes
        );
    }
    let output_end = output_pointer
        .checked_add(output_len)
        .ok_or_else(|| anyhow::anyhow!("plugin output pointer overflowed"))?;
    if output_end > memory.data_size(&store) {
        bail!(
            "plugin returned {output_len} bytes at {output_pointer}, outside its {} byte memory",
            memory.data_size(&store)
        );
    }
    Ok(memory.data(&store)[output_pointer..output_end].to_vec())
}

/// Execute a module with a JSON value and parse its JSON response.
pub fn analyze_json(module_bytes: &[u8], input: &Value) -> Result<Value> {
    analyze_json_with(module_bytes, input, &RuntimeLimits::default())
}

/// Execute a module with an explicit limit set and JSON input/output.
pub fn analyze_json_with(
    module_bytes: &[u8],
    input: &Value,
    limits: &RuntimeLimits,
) -> Result<Value> {
    let input = serde_json::to_vec(input).context("cannot serialize plugin input as JSON")?;
    if input.len() > limits.max_input_bytes {
        bail!(
            "plugin input is too large ({} bytes, limit {})",
            input.len(),
            limits.max_input_bytes
        );
    }
    let output = analyze_with(module_bytes, &input, limits)?;
    if output.len() > limits.max_output_bytes {
        bail!(
            "plugin output is too large ({} bytes, limit {})",
            output.len(),
            limits.max_output_bytes
        );
    }
    serde_json::from_slice(&output).context("plugin returned invalid JSON")
}

fn compile(bytes: &[u8], limits: &RuntimeLimits) -> Result<(Engine, Module, ModuleInfo)> {
    if limits.max_memory_bytes < WASM_PAGE_BYTES {
        bail!("runtime limit max_memory_bytes must be at least one {WASM_PAGE_BYTES}-byte page");
    }
    if bytes.len() > limits.max_module_bytes {
        bail!(
            "Wasm module is too large ({} bytes, limit {})",
            bytes.len(),
            limits.max_module_bytes
        );
    }
    if !bytes.starts_with(b"\0asm") {
        bail!("plugin entry must be a binary WebAssembly module");
    }

    let engine = engine(limits);
    let module =
        Module::new(&engine, bytes).context("invalid or unsupported WebAssembly module")?;

    let imports = module.imports();
    if imports.len() > 0 {
        let first = module
            .imports()
            .next()
            .expect("non-empty import iterator has a first item");
        bail!(
            "Wasm imports are not allowed (found `{}.{}`)",
            first.module(),
            first.name()
        );
    }

    let max_pages = (limits.max_memory_bytes / WASM_PAGE_BYTES) as u64;
    let memory = match module.get_export("memory") {
        Some(ExternType::Memory(memory)) => memory,
        Some(_) => bail!("plugin export `memory` is not linear memory"),
        None => bail!("plugin does not export `memory`"),
    };
    if memory.is_64() {
        bail!("plugin memory must use 32-bit indices");
    }
    if memory.minimum() > max_pages {
        bail!(
            "plugin initial memory of {} pages exceeds the {max_pages}-page limit",
            memory.minimum()
        );
    }
    if let Some(maximum) = memory.maximum()
        && maximum > max_pages
    {
        bail!("plugin maximum memory of {maximum} pages exceeds the {max_pages}-page limit");
    }

    require_func(&module, "amph_abi_version", &[], &[ValType::I32])?;
    require_func(&module, "amph_alloc", &[ValType::I32], &[ValType::I32])?;
    require_func(
        &module,
        "amph_analyze",
        &[ValType::I32, ValType::I32],
        &[ValType::I64],
    )?;

    Ok((
        engine,
        module,
        ModuleInfo {
            abi_version: ABI_VERSION,
            memory_pages: memory.minimum(),
            memory_max_pages: memory.maximum(),
        },
    ))
}

fn require_func(
    module: &Module,
    name: &str,
    params: &[ValType],
    results: &[ValType],
) -> Result<()> {
    match module.get_export(name) {
        Some(ExternType::Func(ty)) => {
            if !func_matches(&ty, params, results) {
                bail!(
                    "plugin export `{name}` has an invalid signature; expected ({}) -> ({})",
                    type_list(params),
                    type_list(results)
                );
            }
            Ok(())
        }
        Some(_) => bail!("plugin export `{name}` is not a function"),
        None => bail!("plugin does not export `{name}`"),
    }
}

fn func_matches(ty: &FuncType, params: &[ValType], results: &[ValType]) -> bool {
    ty.params() == params && ty.results() == results
}

fn type_list(types: &[ValType]) -> String {
    types
        .iter()
        .map(|ty| match ty {
            ValType::I32 => "i32",
            ValType::I64 => "i64",
            ValType::F32 => "f32",
            ValType::F64 => "f64",
            ValType::V128 => "v128",
            ValType::FuncRef => "funcref",
            ValType::ExternRef => "externref",
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn engine(limits: &RuntimeLimits) -> Engine {
    let mut config = Config::default();
    config
        .consume_fuel(true)
        .allow_start_fn(false)
        .wasm_mutable_global(true)
        .wasm_sign_extension(true)
        .wasm_saturating_float_to_int(true)
        .wasm_multi_value(true)
        .wasm_bulk_memory(true)
        .wasm_multi_memory(false)
        .wasm_reference_types(false)
        .wasm_tail_call(false)
        .wasm_extended_const(false)
        .wasm_custom_page_sizes(false)
        .wasm_memory64(false)
        .wasm_wide_arithmetic(false)
        .floats(true)
        .set_max_recursion_depth(limits.max_recursion_depth.max(1));
    config.enforced_limits(EnforcedLimits::strict());
    Engine::new(&config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wasm(wat: &str) -> Vec<u8> {
        wat::parse_str(wat).unwrap()
    }

    fn echo_module() -> Vec<u8> {
        wasm(
            r#"(module
                (memory (export "memory") 1)
                (func (export "amph_abi_version") (result i32) i32.const 1)
                (func (export "amph_alloc") (param i32) (result i32) i32.const 1024)
                (func (export "amph_analyze") (param $ptr i32) (param $len i32) (result i64)
                    (memory.copy (i32.const 2048) (local.get $ptr) (local.get $len))
                    (i64.or
                        (i64.shl (i64.extend_i32_u (i32.const 2048)) (i64.const 32))
                        (i64.extend_i32_u (local.get $len)))))"#,
        )
    }

    fn module_with_analyze(analyze_body: &str) -> Vec<u8> {
        wasm(&format!(
            r#"(module
                (memory (export "memory") 1)
                (func (export "amph_abi_version") (result i32) i32.const 1)
                (func (export "amph_alloc") (param i32) (result i32) i32.const 1024)
                (func (export "amph_analyze") (param i32 i32) (result i64)
                    {analyze_body}))"#
        ))
    }

    #[test]
    fn validates_the_core_abi() {
        let info = validate_module(&echo_module()).unwrap();
        assert_eq!(info.abi_version, ABI_VERSION);
        assert_eq!(info.memory_pages, 1);
        assert_eq!(info.memory_max_pages, None);
    }

    #[test]
    fn analyzes_json_without_persisting_state() {
        let input = json!({"packages": ["bash", "nano"], "score": 0.5});
        let output = analyze_json(&echo_module(), &input).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn rejects_imports_including_wasi() {
        let bytes = wasm(
            r#"(module
                (import "wasi_snapshot_preview1" "proc_exit" (func (param i32)))
                (memory (export "memory") 1)
                (func (export "amph_abi_version") (result i32) i32.const 1)
                (func (export "amph_alloc") (param i32) (result i32) i32.const 0)
                (func (export "amph_analyze") (param i32 i32) (result i64) i64.const 0))"#,
        );
        let error = validate_module(&bytes).unwrap_err().to_string();
        assert!(error.contains("imports are not allowed"), "{error}");
    }

    #[test]
    fn rejects_start_functions() {
        let bytes = wasm(
            r#"(module
                (memory (export "memory") 1)
                (func $start)
                (start $start)
                (func (export "amph_abi_version") (result i32) i32.const 1)
                (func (export "amph_alloc") (param i32) (result i32) i32.const 0)
                (func (export "amph_analyze") (param i32 i32) (result i64) i64.const 0))"#,
        );
        assert!(validate_module(&bytes).is_err());
    }

    #[test]
    fn rejects_missing_or_mistyped_exports() {
        let missing_memory = wasm(
            r#"(module
                (memory 1)
                (func (export "amph_abi_version") (result i32) i32.const 1)
                (func (export "amph_alloc") (param i32) (result i32) i32.const 0)
                (func (export "amph_analyze") (param i32 i32) (result i64) i64.const 0))"#,
        );
        assert!(validate_module(&missing_memory).is_err());

        let wrong_signature = wasm(
            r#"(module
                (memory (export "memory") 1)
                (func (export "amph_abi_version") (result i32) i32.const 1)
                (func (export "amph_alloc") (param i32) (result i32) i32.const 0)
                (func (export "amph_analyze") (param i32) (result i64) i64.const 0))"#,
        );
        let error = validate_module(&wrong_signature).unwrap_err().to_string();
        assert!(error.contains("invalid signature"), "{error}");
    }

    #[test]
    fn rejects_a_wrong_runtime_abi_version() {
        let bytes = wasm(
            r#"(module
                (memory (export "memory") 1)
                (func (export "amph_abi_version") (result i32) i32.const 2)
                (func (export "amph_alloc") (param i32) (result i32) i32.const 1024)
                (func (export "amph_analyze") (param i32 i32) (result i64) i64.const 0))"#,
        );
        // The module shape is valid, but the returned ABI value is not.
        assert!(validate_module(&bytes).is_ok());
        let error = analyze(&bytes, b"{}").unwrap_err().to_string();
        assert!(error.contains("reported ABI 2"), "{error}");
    }

    #[test]
    fn rejects_oversized_memory_and_modules() {
        let large_memory = wasm(
            r#"(module
                (memory (export "memory") 300)
                (func (export "amph_abi_version") (result i32) i32.const 1)
                (func (export "amph_alloc") (param i32) (result i32) i32.const 0)
                (func (export "amph_analyze") (param i32 i32) (result i64) i64.const 0))"#,
        );
        let error = validate_module(&large_memory).unwrap_err().to_string();
        assert!(error.contains("memory"), "{error}");

        let oversized = vec![0; MAX_MODULE_BYTES + 1];
        assert!(validate_module(&oversized).is_err());
    }

    #[test]
    fn rejects_text_modules_in_the_binary_abi() {
        let error = validate_module(b"(module)").unwrap_err().to_string();
        assert!(error.contains("binary WebAssembly"), "{error}");
    }

    #[test]
    fn rejects_out_of_bounds_and_oversized_outputs() {
        let out_of_bounds = module_with_analyze(
            "(i64.or (i64.shl (i64.extend_i32_u (i32.const 65535)) (i64.const 32)) (i64.const 10))",
        );
        let error = analyze(&out_of_bounds, b"{}").unwrap_err().to_string();
        assert!(error.contains("outside its"), "{error}");

        let packed = (2048u64 << 32) | (1024 * 1024 + 1);
        let oversized = module_with_analyze(&format!("i64.const {packed}"));
        let error = analyze(&oversized, b"{}").unwrap_err().to_string();
        assert!(error.contains("too large"), "{error}");
    }

    #[test]
    fn rejects_invalid_json_output() {
        let bytes = wasm(
            r#"(module
                (memory (export "memory") 1)
                (data (i32.const 2048) "not json")
                (func (export "amph_abi_version") (result i32) i32.const 1)
                (func (export "amph_alloc") (param i32) (result i32) i32.const 1024)
                (func (export "amph_analyze") (param i32 i32) (result i64)
                    (i64.or
                        (i64.shl (i64.extend_i32_u (i32.const 2048)) (i64.const 32))
                        (i64.const 7))))"#,
        );
        let error = analyze_json(&bytes, &json!({})).unwrap_err().to_string();
        assert!(error.contains("invalid JSON"), "{error}");
    }

    #[test]
    fn fuel_bounds_non_terminating_modules() {
        let bytes = module_with_analyze("(loop $loop (br $loop)) (i64.const 0)");
        let limits = RuntimeLimits {
            fuel: 10_000,
            ..RuntimeLimits::default()
        };
        let error = format!("{:#}", analyze_with(&bytes, b"{}", &limits).unwrap_err());
        assert!(
            error.to_lowercase().contains("fuel") || error.to_lowercase().contains("out of fuel"),
            "{error}"
        );
    }

    #[test]
    fn input_size_is_bounded() {
        let limits = RuntimeLimits {
            max_input_bytes: 4,
            ..RuntimeLimits::default()
        };
        let error = analyze_json_with(&echo_module(), &json!({"too": "large"}), &limits)
            .unwrap_err()
            .to_string();
        assert!(error.contains("input is too large"), "{error}");
    }
}
