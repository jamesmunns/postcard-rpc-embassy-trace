#![no_std]
#![no_main]

use app::{AppTx, LoadState};
use defmt::info;
#[cfg(feature = "drs-scheduler")]
use embassy_executor::raw::Deadline;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_nrf::{
    bind_interrupts,
    config::{Config as NrfConfig, HfclkSource},
    gpio::{Level, Output, OutputDrive},
    pac::FICR,
    peripherals::USBD,
    usb::{self, vbus_detect::HardwareVbusDetect},
};
use embassy_time::{Duration, Instant, Ticker, Timer};
use embassy_usb::{Config, UsbDevice};
use load::{worker, HALT, STAGE};
use postcard_rpc::{
    sender_fmt,
    server::{Dispatch, Sender, Server},
};
use static_cell::StaticCell;
use template_icd::StageCommand;
use trace::{drain, task_identify};

bind_interrupts!(pub struct Irqs {
    USBD => usb::InterruptHandler<USBD>;
    CLOCK_POWER => usb::vbus_detect::InterruptHandler;
});

use {defmt_rtt as _, panic_probe as _};

pub mod app;
pub mod handlers;
pub mod load;
pub mod trace;

fn usb_config(serial: &'static str) -> Config<'static> {
    let mut config = Config::new(0x16c0, 0x27DD);
    config.manufacturer = Some("OneVariable");
    config.product = Some("poststation-nrf");
    config.serial_number = Some(serial);

    // Required for windows compatibility.
    // https://developer.nordicsemi.com/nRF_Connect_SDK/doc/1.9.1/kconfig/CONFIG_CDC_ACM_IAD.html#help
    config.device_class = 0xEF;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;
    config.composite_with_iads = true;

    config
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // SYSTEM INIT
    info!("Start");
    let mut config = NrfConfig::default();
    config.hfclk_source = HfclkSource::ExternalXtal;
    let p = embassy_nrf::init(Default::default());
    // Obtain the device ID
    let unique_id = get_unique_id();

    static SERIAL_STRING: StaticCell<[u8; 16]> = StaticCell::new();
    let mut ser_buf = [b' '; 16];
    // This is a simple number-to-hex formatting
    unique_id
        .to_be_bytes()
        .iter()
        .zip(ser_buf.chunks_exact_mut(2))
        .for_each(|(b, chs)| {
            let mut b = *b;
            for c in chs {
                *c = match b >> 4 {
                    v @ 0..10 => b'0' + v,
                    v @ 10..16 => b'A' + (v - 10),
                    _ => b'X',
                };
                b <<= 4;
            }
        });
    let ser_buf = SERIAL_STRING.init(ser_buf);
    let ser_buf = core::str::from_utf8(ser_buf.as_slice()).unwrap();

    // Spawn a worker task to ensure that all the pool is initialized
    Timer::after_millis(100).await;
    spawner.must_spawn(worker(StageCommand {
        ident: 0,
        steps: heapless::Vec::new(),
        loops: false,
        deadline_ticks: 100,
        loop_delay_ticks: 0,
        start_delay_ticks: 0,
    }));
    Timer::after_millis(100).await;
    STAGE.wake_all();
    HALT.wake_all();

    // USB/RPC INIT
    let driver = usb::Driver::new(p.USBD, Irqs, HardwareVbusDetect::new(Irqs));
    let pbufs = app::PBUFS.take();
    let config = usb_config(ser_buf);
    let led = Output::new(p.P0_13, Level::Low, OutputDrive::Standard);

    let context = app::Context {
        unique_id,
        led,
        load_state: LoadState::Idle,
        spawner,
    };

    let (device, tx_impl, rx_impl) =
        app::STORAGE.init_poststation(driver, config, pbufs.tx_buf.as_mut_slice());
    tx_impl.set_timeout_ms_per_frame(500).await;
    let dispatcher = app::MyApp::new(context, spawner.into());
    let vkk = dispatcher.min_key_len();
    let mut server: app::AppServer = Server::new(
        tx_impl,
        rx_impl,
        pbufs.rx_buf.as_mut_slice(),
        dispatcher,
        vkk,
    );
    let sender = server.sender();
    // We need to spawn the USB task so that USB messages are handled by
    // embassy-usb
    spawner.must_spawn(drain(sender.clone()));
    spawner.must_spawn(usb_task(device));
    spawner.must_spawn(logging_task(sender));

    // for _ in 0..5 {
    //     spawner.must_spawn(looper());
    // }

    // Begin running!
    #[cfg(feature = "drs-scheduler")]
    Deadline::set_current_task_deadline(1).await;

    let work_fut = async move {
        loop {
            // If the host disconnects, we'll return an error here.
            // If this happens, just wait until the host reconnects
            let _x = server.run().await;
            defmt::info!("I/O error");
            Timer::after_millis(100).await;
        }
    };
    let ident_fut = async move {
        let mut ticker = Ticker::every(Duration::from_secs(3));
        loop {
            ticker.next().await;
            task_identify("MAIN_TASK").await;
        }
    };

    join(work_fut, ident_fut).await;

}

/// This handles the low level USB management
#[embassy_executor::task]
pub async fn usb_task(mut usb: UsbDevice<'static, app::AppDriver>) {
    // lol make this the highest prio
    #[cfg(feature = "drs-scheduler")]
    Deadline::set_current_task_deadline(0).await;

    let run_fut = usb.run();
    let ident_fut = async move {
        let mut ticker = Ticker::every(Duration::from_secs(3));
        loop {
            ticker.next().await;
            task_identify("USB_TASK").await;
        }
    };

    join(run_fut, ident_fut).await;
}

/// This task is a "sign of life" logger
#[embassy_executor::task]
pub async fn logging_task(sender: Sender<AppTx>) {
    let mut ticker = Ticker::every(Duration::from_secs(3));
    let start = Instant::now();
    loop {
        ticker.next().await;
        task_identify("LOG_TASK").await;
        let _ = sender_fmt!(sender, "Uptime: {:?}", start.elapsed()).await;
    }
}

// #[embassy_executor::task(pool_size = 5)]
// pub async fn looper() {
//     let mut ticker = Ticker::every(Duration::from_millis(100));
//     loop {
//         ticker.next().await;
//         let now = Instant::now();
//         while now.elapsed() < Duration::from_millis(10) {
//             // busy loop!
//         }
//     }
// }

fn get_unique_id() -> u64 {
    let lower = FICR.deviceid(0).read() as u64;
    let upper = FICR.deviceid(1).read() as u64;
    (upper << 32) | lower
}
