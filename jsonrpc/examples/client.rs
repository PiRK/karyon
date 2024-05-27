use std::time::Duration;

use serde::{Deserialize, Serialize};
use smol::Timer;

use karyon_jsonrpc::Client;

#[derive(Deserialize, Serialize)]
struct Req {
    x: u32,
    y: u32,
}

#[derive(Deserialize, Serialize, Debug)]
struct Pong {}

fn main() {
    env_logger::init();
    smol::future::block_on(async {
        let client = Client::builder("tcp://127.0.0.1:6000")
            .expect("Create client builder")
            .build()
            .await
            .unwrap();

        let clientc = client.clone();
        smol::spawn(async move {
            loop {
                Timer::after(Duration::from_millis(500)).await;
                let result: Pong = clientc.call("Calc.ping", ()).await.unwrap();
                println!("ping msg result:  {:?}", result);
            }
        })
        .detach();

        let params = Req { x: 10, y: 7 };
        let result: u32 = client.call("Calc.add", params).await.unwrap();
        println!("result {result}");

        let params = Req { x: 10, y: 7 };
        let result: u32 = client.call("Calc.sub", params).await.unwrap();
        println!("result {result}");

        let result: String = client.call("Calc.version", ()).await.unwrap();
        println!("result {result}");

        Timer::after(Duration::from_secs(10)).await;
    });
}
