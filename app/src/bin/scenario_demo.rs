use std::time::Duration;

use poststation_sdk::connect_insecure;
use template_icd::{HaltTasksEndpoint, StageCommand, StageTaskEndpoint, StartTasksEndpoint, Step};
use tokio::time::sleep;

use tikv_jemallocator::Jemalloc;
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() -> Result<(), String> {
    inner_main().await
}

const SERIAL: u64 = 0x4274E020EBB2D27E;

async fn inner_main() -> Result<(), String> {
    let client = connect_insecure(51837).await.unwrap();
    sleep(Duration::from_secs(1)).await;

    println!("Halting...");
    let _ = client.proxy_endpoint::<HaltTasksEndpoint>(
        SERIAL,
        200,
        &()
    ).await.unwrap();
    sleep(Duration::from_millis(1000)).await;


    for i in 100..140 {
        let mut steps = heapless::Vec::new();
        let _ = steps.push(Step::WorkTicks { ticks: 32768 / 2000 });
        let _ = steps.push(Step::Yield);
        let _ = steps.push(Step::WorkTicks { ticks: 32768 / 2000 });
        let _ = steps.push(Step::Yield);
        let _ = steps.push(Step::WorkTicks { ticks: 32768 / 2000 });
        let _ = steps.push(Step::Yield);
        let _ = steps.push(Step::WorkTicks { ticks: 32768 / 2000 });
        let _ = steps.push(Step::SleepTicks { ticks: 32768 / 4 });

        let _ = steps.push(Step::WorkTicks { ticks: 32768 / 2000 });
        let _ = steps.push(Step::Yield);
        let _ = steps.push(Step::WorkTicks { ticks: 32768 / 2000 });
        let _ = steps.push(Step::Yield);
        let _ = steps.push(Step::WorkTicks { ticks: 32768 / 2000 });
        let _ = steps.push(Step::Yield);
        let _ = steps.push(Step::WorkTicks { ticks: 32768 / 2000 });
        let _ = steps.push(Step::SleepTicks { ticks: 32768 / 4 });


        client.proxy_endpoint::<StageTaskEndpoint>(
            SERIAL,
            i,
            &StageCommand {
                ident: i,
                steps,
                loops: true, // i % 2 == 0,
                deadline_ticks: 16600, // if i % 2 == 0 { 30000 } else { 16600 },
                loop_delay_ticks: 0,
                start_delay_ticks: i as u64 * 150, // if i % 2 == 0 { i as u64 * 150 } else { 32768 * 3 },
            }
        ).await.unwrap().unwrap();
    }

    println!("Halted, Starting in 5s:");
    sleep(Duration::from_millis(5000)).await;
    client.proxy_endpoint::<StartTasksEndpoint>(
        SERIAL,
        200,
        &()
    ).await.unwrap().unwrap();

    println!("Running...");
    loop {
        sleep(Duration::from_millis(1000)).await;
    }
}

