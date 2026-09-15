fn main() {
    println!("cargo:rerun-if-env-changed=WIND_OUTTO_INSTALLER");
    if std::env::var_os("CARGO_FEATURE_EMBEDDED_PAYLOAD").is_some() {
        let path = std::env::var("WIND_OUTTO_INSTALLER")
            .expect("WIND_OUTTO_INSTALLER must name the built outto installer");
        println!("cargo:rerun-if-changed={path}");
        assert!(
            std::path::Path::new(&path).is_file(),
            "missing outto payload"
        );
    }
}
