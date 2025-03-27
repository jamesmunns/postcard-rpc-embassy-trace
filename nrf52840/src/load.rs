use core::pin::pin;

use embassy_futures::{select::{select, Either}, yield_now};
use embassy_time::{Instant, Timer};
use template_icd::{StageCommand, Step};
use maitake_sync::WaitQueue;

use crate::trace::{deadline_start, deadline_stop, task_identify};

#[cfg(feature = "drs-scheduler")]
use embassy_executor::raw::Deadline;


pub static STAGE: WaitQueue = WaitQueue::new();
pub static HALT: WaitQueue = WaitQueue::new();
pub static TRIGGER: WaitQueue = WaitQueue::new();

#[embassy_executor::task(pool_size = 50)]
pub async fn worker(cmd: StageCommand) {
    // Avoid infinite loops
    if cmd.steps.is_empty() && cmd.loops {
        panic!();
    }

    let task_id = {
        use core::fmt::Write;
        let mut name = heapless::String::<10>::new();
        let _ = name.push_str("WRK-");
        let _ = write!(&mut name, "{:06X}", cmd.ident);
        task_identify(name.as_str()).await
    };
    let halt_fut = HALT.wait();
    let mut halt_fut = pin!(halt_fut);
    let _ = halt_fut.as_mut().subscribe();

    let stage_fut = STAGE.wait();
    let mut stage_fut = pin!(stage_fut);
    let _ = stage_fut.as_mut().subscribe();

    let run_fut = async {
        // Wait for the trigger
        let _ = stage_fut.await;

        // Schedule ourselves for wakeup when the delay is over
        #[cfg(feature = "drs-scheduler")]
        Deadline::set_current_task_deadline_after(cmd.start_delay_ticks).await;
        Timer::after_ticks(cmd.start_delay_ticks).await;

        #[cfg(feature = "drs-scheduler")]
        Deadline::set_current_task_deadline_after(cmd.deadline_ticks).await;

        loop {
            if !cmd.manual_start_stop {
                deadline_start(task_id, cmd.deadline_ticks);
            }
            for step in cmd.steps.iter() {
                match step {
                    Step::SleepTicks { ticks } => Timer::after_ticks((*ticks).into()).await,
                    Step::WorkTicks { ticks } => {
                        let now = Instant::now();
                        let ttl = u64::from(*ticks);
                        while now.elapsed().as_ticks() < ttl {
                            // busy!
                        }
                    },
                    Step::Yield => {
                        yield_now().await;
                    },
                    Step::WaitTrigger => {
                        let _ = TRIGGER.wait().await;
                    },
                    Step::WakeTrigger => {
                        TRIGGER.wake_all();
                    },
                    Step::ManualDeadlineStart => {
                        assert!(cmd.manual_start_stop);
                        deadline_start(task_id, cmd.deadline_ticks);
                        yield_now().await;
                    }
                    Step::ManualDeadlineStop => {
                        assert!(cmd.manual_start_stop);
                        deadline_stop(task_id);
                    }
                }
            }
            if !cmd.manual_start_stop {
                deadline_stop(task_id);
            }

            if !cmd.loops {
                break;
            } else {
                Timer::after_ticks(cmd.loop_delay_ticks).await;
                #[cfg(feature = "drs-scheduler")]
                Deadline::set_current_task_deadline_after(cmd.deadline_ticks).await;
            }
        }
    };

    match select(run_fut, halt_fut).await {
        Either::First(_) => defmt::info!("Worker task {=u32} completed", cmd.ident),
        Either::Second(_) => {
            defmt::info!("Worker task {=u32} halted", cmd.ident);
            deadline_stop(task_id);
        },
    }
}
