use matchbox_socket::WebRtcSocketBuilder;
fn main() {
    let builder = WebRtcSocketBuilder::new("wss://example.com");
    // let _ = builder.add_ice_server("turn:example.com");
}
