use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=ASC_MEDIA_SOURCE_DLL");
    println!("cargo:rerun-if-env-changed=ASC_CROSS_COMPILE_RESOURCES");
    let destination = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set"))
        .join("AutomaticScreenCameraSource.dll");
    if let Some(source) = env::var_os("ASC_MEDIA_SOURCE_DLL") {
        println!(
            "cargo:rerun-if-changed={}",
            PathBuf::from(&source).display()
        );
        fs::copy(&source, &destination).unwrap_or_else(|error| {
            panic!(
                "could not embed media-source DLL {}: {error}",
                PathBuf::from(source).display()
            )
        });
    } else {
        fs::write(destination, []).expect("could not create empty development payload");
    }

    let windows_target = env::var("CARGO_CFG_TARGET_OS").is_ok_and(|target| target == "windows");
    let windows_host = env::var("HOST").is_ok_and(|host| host.contains("windows"));
    let cross_compile_resources = env::var_os("ASC_CROSS_COMPILE_RESOURCES").is_some();
    if windows_target && (windows_host || cross_compile_resources) {
        let manifest = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false" /></requestedPrivileges></security>
  </trustInfo>
  <application xmlns="urn:schemas-microsoft-com:asm.v3"><windowsSettings>
    <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
    <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
  </windowsSettings></application>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1"><application>
    <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}" />
  </application></compatibility>
</assembly>"#;
        winresource::WindowsResource::new()
            .set_manifest(manifest)
            .compile()
            .expect("could not compile Windows manifest and version resources");
    }
}
