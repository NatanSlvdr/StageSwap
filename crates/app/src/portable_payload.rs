pub fn bytes() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/AutomaticScreenCameraSource.dll"))
}
