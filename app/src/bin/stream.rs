use demo::{ExecData, ExecState, System};
use postcard::accumulator::{CobsAccumulator, FeedResult};
use poststation_sdk::connect_insecure;
use template_icd::{Event, TraceReportTopic};

use tikv_jemallocator::Jemalloc;
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() -> Result<(), String> {
    tokio::task::spawn(inner_main()).await.unwrap().unwrap();
    Ok(())
}


async fn inner_main() -> Result<(), String> {
    const SERIAL: u64 = 0x4274E020EBB2D27E;
    let client = connect_insecure(51837).await.unwrap();
    let mut sub = client
        .stream_topic::<TraceReportTopic>(SERIAL)
        .await
        .unwrap();
    let mut system = System::default();

    let mut acc = CobsAccumulator::<64>::new();

    loop {
        let msg = sub.recv().await.unwrap();

        let mut window = msg.events_cobs.as_slice();
        'cobs: while !window.is_empty() {
            window = match acc.feed_ref::<Event>(window) {
                FeedResult::Consumed => break 'cobs,
                FeedResult::OverFull(_new_wind) => todo!(),
                FeedResult::DeserError(new_wind) => {
                    println!("WARN: Decode Error");
                    new_wind
                },
                FeedResult::Success {
                    data: evt,
                    remaining,
                } => {
                    system.handle_evt(evt, window.len() - remaining.len());
                    remaining
                }
            };
        }

        system.check_render();
    }
}
