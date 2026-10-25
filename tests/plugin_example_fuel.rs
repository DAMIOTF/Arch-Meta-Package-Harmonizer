#[test]
fn example_analyzer_handles_a_thousand_package_snapshot_with_default_fuel() {
    let module = include_bytes!("fixtures/wasm/amph-example-plugin-v1.0.1.wasm");
    assert_eq!(
        amph::plugin::wasm::validate_module(module)
            .expect("the published example must match the AMPH Wasm subset")
            .abi_version,
        1
    );

    let packages = (0..1000usize)
        .map(|index| {
            serde_json::json!({
                "name": format!("package-{index}"),
                "version": "1.2.3-1",
                "size": 1_048_576,
                "explicit": index % 4 == 0,
                "foreign": index % 17 == 0,
                "depends": ["glibc", "gcc-libs", "openssl", "zlib", "systemd-libs"],
                "provides": [format!("virtual-{index}"), format!("abi-{index}")]
            })
        })
        .collect::<Vec<_>>();
    let request = serde_json::json!({
        "api": 1,
        "op": "analyze",
        "snapshot": {"scanned_at": "now", "packages": packages}
    });

    let result = amph::plugin::wasm::analyze_json(module, &request)
        .expect("the example should analyze a realistic inventory with default limits");
    assert_eq!(result["findings"][0]["packages"], 1000);
}
