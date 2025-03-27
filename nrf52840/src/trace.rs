// Implementations of the embassy trace features

use core::{future::poll_fn, pin::pin, sync::atomic::{AtomicUsize, Ordering}, task::Poll};

use critical_section::CriticalSection;
use embassy_executor::raw::task_from_waker;
use bbq2::nicknames::Churrasco;
use embassy_time::{Duration, Instant, WithTimeout};
use grounded::uninit::GroundedCell;
use maitake_sync::WaitQueue;
use postcard_rpc::{header::VarSeq, server::Sender};
use template_icd::{Event, Report, TraceReportTopic};
use postcard::to_slice_cobs;
#[cfg(feature = "drs-scheduler")]
use embassy_executor::raw::Deadline;

use crate::app::AppTx;

const SIZE: usize = 64 * 1024;
const NOTIF_SIZE: usize = 768 * 4;
static QUEUE: Churrasco<SIZE> = Churrasco::new();
static PENDING_Q: WaitQueue = WaitQueue::new();
static LAST: GroundedCell<u64> = GroundedCell::const_init();

#[inline]
fn now_delta(_cs: CriticalSection) -> u64 {
    let last = unsafe { *LAST.get() };
    Instant::now().as_ticks() - last
}

#[inline]
fn now_set(_cs: CriticalSection) -> u64 {
    let now = Instant::now().as_ticks();
    unsafe { *LAST.get() = now; };
    now
}

#[inline]
fn now() -> u64 {
    Instant::now().as_ticks()
}

#[inline]
fn trace(evt: &Event, _cs: CriticalSection) {
    static PENDING: AtomicUsize = AtomicUsize::new(0);
    let prod = QUEUE.stream_producer();

    // Ideally, we wouldn't need a critical section, however it seems that
    // we occasionally have an interrupt that pends a task while a
    // grant is already being serviced. To avoid, this, we take a short
    // critical section to ensure there is not contention when sending
    // the data.
    let (res, used) = {
        if let Ok(mut wgr) = prod.grant_exact(32) {
            let used = to_slice_cobs(evt, &mut wgr).unwrap();
            let len = used.len();
            wgr.commit(len);
            (true, len)
        } else {
            (false, 0)
        }
    };

    // Increment the amount pending
    let old = PENDING.load(Ordering::Relaxed);
    let new = old + used;
    if new > NOTIF_SIZE {
        PENDING.store(0, Ordering::Relaxed);
        PENDING_Q.wake_all();
    } else {
        PENDING.store(new, Ordering::Relaxed);
    }

    if !res {
        // defmt::error!("Oops");
        panic!("Overflowed trace data!");
    }

}

#[no_mangle]
pub extern "Rust" fn _embassy_trace_task_new(_executor_id: u32, task_id: u32) {
    critical_section::with(|cs| {
        let evt = Event::TaskNew { tick: now(), task_id: task_id - 0x20000000 };
        trace(&evt, cs);
    });
}

#[no_mangle]
pub extern "Rust" fn _embassy_trace_task_exec_begin(_executor_id: u32, task_id: u32) {
    critical_section::with(|cs| {
        let evt = Event::TaskExecBegin { tick: now_delta(cs), task_id: task_id - 0x20000000 };
        trace(&evt, cs);
    });
}

#[no_mangle]
pub extern "Rust" fn _embassy_trace_task_exec_end(_executor_id: u32, task_id: u32) {
    critical_section::with(|cs| {
        let evt = Event::TaskExecEnd { tick: now_delta(cs), task_id: task_id - 0x20000000 };
        trace(&evt, cs);
    });
}

#[no_mangle]
pub extern "Rust" fn _embassy_trace_task_ready_begin(_executor_id: u32, task_id: u32) {
    critical_section::with(|cs| {
        let evt = Event::TaskReadyBegin { tick: now_delta(cs), task_id: task_id - 0x20000000 };
        trace(&evt, cs);
    });
}

#[no_mangle]
pub extern "Rust" fn _embassy_trace_executor_idle(_executor_id: u32) {
    critical_section::with(|cs| {
        let evt = Event::ExecutorIdle { tick: now_delta(cs) };
        trace(&evt, cs);
    });
}

#[no_mangle]
pub extern "Rust" fn _embassy_trace_poll_start(_executor_id: u32) {
    critical_section::with(|cs| {
        let evt = Event::ExecutorPollStart { tick: now_set(cs) };
        trace(&evt, cs);
    });
}

pub async fn task_identify(name: &str) -> u32 {
    let task_id: u32 = poll_fn(|cx| {
        Poll::Ready(task_from_waker(cx.waker()).as_id())
    }).await;
    critical_section::with(|cs| {
        let evt = Event::TaskIdentify { tick: now_delta(cs), task_id: task_id - 0x20000000, name };
        trace(&evt, cs);
    });
    task_id
}

pub fn deadline_start(task_id: u32, deadline: u64) {
    critical_section::with(|cs| {
        let evt = Event::DeadlineStart { tick: now_delta(cs), task_id: task_id - 0x20000000, deadline };
        trace(&evt, cs);
    });
}

pub fn deadline_stop(task_id: u32) {
    critical_section::with(|cs| {
        let evt = Event::DeadlineStop { tick: now_delta(cs), task_id: task_id - 0x20000000 };
        trace(&evt, cs);
    });
}

// use embassy_futures::join::join;

#[embassy_executor::task]
pub async fn drain(sender: Sender<AppTx>) {
    #[cfg(feature = "drs-scheduler")]
    Deadline::set_current_task_deadline(2).await;

    // We need to ignore the current task, because USB sends data in chunks
    // of 64 bytes, and if we need to yield, notify, and run for each packet,
    // we are creating a decent number of trace events for each packet. This
    // might be more possible to handle on a USB-HS device where packets are
    // larger than 64 bytes, or if we come up with a more efficient trace
    // encoding.
    // DRAIN_ID.store(task_id, Ordering::Relaxed);
    let cons = QUEUE.stream_consumer();
    let mut ttl = 0usize;
    let mut last = Instant::now();
    let mut ctr = 0u32;
    let mut was_good = false;
    let mut drains = 0usize;
    let start = Instant::now();

    loop {
        let notif = PENDING_Q.wait();
        let mut notif = pin!(notif);
        let fpoll = notif.as_mut().subscribe();

        'inner: loop {
            let Ok(rgr) = cons.read() else {
                break 'inner;
            };

            // HACK: allow USB some time to wake up
            // defmt::info!("PEND: {=usize}", rgr.len());
            if start.elapsed() < Duration::from_secs(3) {
                let len = rgr.len();
                rgr.release(len);
                continue 'inner;
            }

            let len = rgr.len().min(768);
            let sli = &rgr[..len];
            ttl = ttl.wrapping_add(len);
            drains = drains.wrapping_add(1);

            match sender.publish::<TraceReportTopic>(
                VarSeq::Seq4(ctr),
                &Report { events_cobs: sli },
            ).await {
                Ok(_) => {
                    was_good = true;
                },
                Err(e) => {
                    if was_good {
                        panic!("{e:?}");
                    }
                },
            }
            ctr = ctr.wrapping_add(1);
            rgr.release(len);

            if last.elapsed() >= Duration::from_secs(1) {
                last = Instant::now();
                task_identify("DRAIN_TASK").await;
                defmt::info!("-->DG: {=usize} -> {=usize}", ttl, drains);
                ttl = 0;
                drains = 0;
            }

            if len != 768 {
                break 'inner;
            }
        }

        // We don't care why, but if the timeout or a notification occurred, go around
        if fpoll == Poll::Pending {
            let _ = notif.with_timeout(Duration::from_millis(250)).await;
        }
    }
}

// loop {
//     let Ok(rgr) = cons.read() else {
//         Timer::after_millis(20).await;
//         continue;
//     };
//     let len = rgr.len().min(768);
//     let sli = &rgr[..len];
//     ttl = ttl.wrapping_add(len);
//     match sender.publish::<TraceReportTopic>(
//         VarSeq::Seq4(ctr),
//         &Report { events_cobs: sli },
//     ).await {
//         Ok(_) => {
//             was_good = true;
//         },
//         Err(e) => {
//             if was_good {
//                 panic!("{e:?}");
//             }
//         },
//     }
//     ctr = ctr.wrapping_add(1);

//     if last.elapsed() >= Duration::from_secs(1) {
//         last = Instant::now();
//         defmt::info!("-->DG: {=usize}", ttl);
//         ttl = 0;
//     }

//     rgr.release(len);
//     // Write buffer is 1K, don't overflow
//     if len != 768 {
//         Timer::after_millis(20).await;
//     } else {
//         yield_now().await;
//     }
// }
