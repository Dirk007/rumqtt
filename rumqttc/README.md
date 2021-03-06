# rumqttc

[![crates.io page](https://img.shields.io/crates/v/rumqttc.svg)](https://crates.io/crates/rumqttc)
[![docs.rs page](https://docs.rs/rumqttc/badge.svg)](https://docs.rs/rumqttc)

A pure rust MQTT client which strives to be robust, efficient and easy to use.
This library is backed by an async (tokio) eventloop which handles all the
robustness and and efficiency parts of MQTT but naturally fits into both sync
and async worlds as we'll see

Let's jump into examples right away

A simple synchronous publish and subscribe
----------------------------

```rust
use rumqttc::{MqttOptions, Client, QoS};
use std::time::Duration;
use std::thread;

fn main() {
    let mut mqttoptions = MqttOptions::new("rumqtt-sync", "test.mosquitto.org", 1883);
    mqttoptions.set_keep_alive(5);

    let (mut client, mut connection) = Client::new(mqttoptions, 10);
    client.subscribe("hello/rumqtt", QoS::AtMostOnce).unwrap();
    thread::spawn(move || for i in 0..10 {
       client.publish("hello/rumqtt", QoS::AtLeastOnce, false, vec![i; i as usize]).unwrap();
       thread::sleep(Duration::from_millis(100));
    });

    // Iterate to poll the eventloop for connection progress
    for (i, notification) in connection.iter().enumerate() {
        println!("Notification = {:?}", notification);
    }
}
```
A simple asynchronous publish and subscribe
------------------------------
```rust
use rumqttc::{MqttOptions, AsyncClient, QoS};
use tokio::{task, time};
use std::time::Duration;
use std::error::Error;

#[tokio::main(core_threads = 1)]
async fn main() {
    let mut mqttoptions = MqttOptions::new("rumqtt-async", "test.mosquitto.org", 1883);
    mqttoptions.set_keep_alive(5);

    let (mut client, mut eventloop) = AsyncClient::new(mqttoptions, 10);
    client.subscribe("hello/rumqtt", QoS::AtMostOnce).await.unwrap();

    task::spawn(async move {
        for i in 0..10 {
            client.publish("hello/rumqtt", QoS::AtLeastOnce, false, vec![i; i as usize]).await.unwrap();
            time::delay_for(Duration::from_millis(100)).await;
        }
    });

    loop {
        let notification = eventloop.poll().await.unwrap();
        println!("Received = {:?}", notification);
        tokio::time::delay_for(Duration::from_secs(1)).await;
    }
}
```

Quick overview of features
- Eventloop orchestrates outgoing/incoming packets concurrently and hadles the state
- Pings the broker when necessary and detects client side half open connections as well
- Throttling of outgoing packets (todo)
- Queue size based flow control on outgoing packets
- Automatic reconnections by just continuing the `eventloop.poll()/connection.iter()` loop`
- Natural backpressure to client APIs during bad network
- Immediate cancellation with `client.cancel()`

In short, everything necessary to maintain a robust connection

**NOTE**: Looping on `connection.iter()/eventloop.poll()` is necessary to
run the eventloop and make progress. It yields incoming and outgoing activity
notifications which allows customization as user sees fit.

**IMPORTANT** Blocking inside `eventloop.poll()/eonnection.iter()` loop
will block connection progress

Since the eventloop is externally polled (with `iter()/poll()` in a loop)
out side the library and `Eventloop` is accessible, users can
- Distribute incoming messages based on topics
- Stop it when required
- Access internal state for use cases like graceful shutdown or to modify options before reconnection

License: Apache-2.0
