//! Checks that Steam (as Spacewar, App ID 480) works on this computer:
//! `cargo run --example steam_probe`. Needs Steam running and
//! steam_api64.dll next to the example (target/debug/examples).

use std::time::{Duration, Instant};

use steamworks::networking_types::{NetworkingIdentity, SendFlags};
use steamworks::Client;

fn main() {
    let client = match Client::init_app(480) {
        Ok(c) => c,
        Err(e) => {
            println!("PROBE init failed: {e}");
            return;
        }
    };
    let me = client.user().steam_id();
    println!("PROBE signed in as {:?} ({})", client.friends().name(), me.raw());
    let set = client.friends().set_rich_presence("connect", Some(&format!("+ozland_host {}", me.raw())));
    println!("PROBE rich presence set: {set}");

    // Send a message to ourselves through Steam's networking.
    let messages = client.networking_messages();
    messages.session_request_callback(|r| {
        r.accept();
    });
    let start = Instant::now();
    let mut got = false;
    while start.elapsed() < Duration::from_secs(8) && !got {
        client.run_callbacks();
        let _ = messages.send_message_to_user(
            NetworkingIdentity::from(me),
            SendFlags::UNRELIABLE_NO_NAGLE | SendFlags::AUTO_RESTART_BROKEN_SESSION,
            b"hello",
            0,
        );
        for m in messages.receive_messages_on_channel(0, 16) {
            println!("PROBE received {:?} after {:.1?}", String::from_utf8_lossy(m.data()), start.elapsed());
            got = true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !got {
        println!("PROBE no message came back to ourselves (Steam may not loop back; a second account is needed)");
    }
    client.friends().set_rich_presence("connect", None);
}
