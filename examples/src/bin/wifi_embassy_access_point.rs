//! Embassy access point
//!
//! - creates an open access-point with SSID `esp-wifi`
//! - you can connect to it using a static IP in range 192.168.2.2 .. 192.168.2.255, gateway 192.168.2.1
//! - open http://192.168.2.1:8080/ in your browser - the example will perform an HTTP get request to some "random" server
//!
//! On Android you might need to choose _Keep Accesspoint_ when it tells you the WiFi has no internet connection, Chrome might not want to load the URL - you can use a shell and try `curl` and `ping`
//!
//! Because of the huge task-arena size configured this won't work on ESP32-S2
//! When using USB-SERIAL-JTAG you may have to activate the feature `phy-enable-usb` in the esp-wifi crate.

//% FEATURES: embassy embassy-generic-timers esp-wifi esp-wifi/async esp-wifi/embassy-net esp-wifi/wifi-default esp-wifi/wifi esp-wifi/utils
//% CHIPS: esp32 esp32s2 esp32s3 esp32c2 esp32c3 esp32c6

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_net::{
    tcp::TcpSocket,
    IpListenEndpoint,
    Ipv4Address,
    Ipv4Cidr,
    Stack,
    StackResources,
    StaticConfigV4,
};
use embassy_time::{Duration, Timer};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{prelude::*, rng::Rng, timer::timg::TimerGroup};
use esp_println::{print, println};
use esp_wifi::{
    init,
    wifi::{
        AccessPointConfiguration,
        Configuration,
        WifiApDevice,
        WifiController,
        WifiDevice,
        WifiEvent,
        WifiState,
    },
    EspWifiInitFor,
};

// When you are okay with using a nightly compiler it's better to use https://docs.rs/static_cell/2.1.0/static_cell/macro.make_static.html
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let peripherals = esp_hal::init({
        let mut config = esp_hal::Config::default();
        config.cpu_clock = CpuClock::max();
        config
    });

    esp_alloc::heap_allocator!(72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);

    let init = init(
        EspWifiInitFor::Wifi,
        timg0.timer0,
        Rng::new(peripherals.RNG),
        peripherals.RADIO_CLK,
    )
    .unwrap();

    let wifi = peripherals.WIFI;
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiApDevice).unwrap();

    cfg_if::cfg_if! {
        if #[cfg(feature = "esp32")] {
            let timg1 = TimerGroup::new(peripherals.TIMG1);
            esp_hal_embassy::init(timg1.timer0);
        } else {
            use esp_hal::timer::systimer::{SystemTimer, Target};
            let systimer = SystemTimer::new(peripherals.SYSTIMER).split::<Target>();
            esp_hal_embassy::init(systimer.alarm0);
        }
    }

    let config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(192, 168, 2, 1), 24),
        gateway: Some(Ipv4Address::from_bytes(&[192, 168, 2, 1])),
        dns_servers: Default::default(),
    });

    let seed = 1234; // very random, very secure seed

    // Init network stack
    let stack = &*mk_static!(
        Stack<WifiDevice<'_, WifiApDevice>>,
        Stack::new(
            wifi_interface,
            config,
            mk_static!(StackResources<3>, StackResources::<3>::new()),
            seed
        )
    );

    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(&stack)).ok();

    let mut rx_buffer = [0; 1536];
    let mut tx_buffer = [0; 1536];

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
    println!("Connect to the AP `esp-wifi` and point your browser to http://192.168.2.1:8080/");
    println!("Use a static IP in the range 192.168.2.2 .. 192.168.2.255, use gateway 192.168.2.1");

    let mut socket = TcpSocket::new(&stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));
    loop {
        println!("Wait for connection...");
        let r = socket
            .accept(IpListenEndpoint {
                addr: None,
                port: 8080,
            })
            .await;
        println!("Connected...");

        if let Err(e) = r {
            println!("connect error: {:?}", e);
            continue;
        }

        use embedded_io_async::Write;

        let mut buffer = [0u8; 1024];
        let mut pos = 0;
        loop {
            match socket.read(&mut buffer).await {
                Ok(0) => {
                    println!("read EOF");
                    break;
                }
                Ok(len) => {
                    let to_print =
                        unsafe { core::str::from_utf8_unchecked(&buffer[..(pos + len)]) };

                    if to_print.contains("\r\n\r\n") {
                        print!("{}", to_print);
                        println!();
                        break;
                    }

                    pos += len;
                }
                Err(e) => {
                    println!("read error: {:?}", e);
                    break;
                }
            };
        }

        let r = socket
            .write_all(
                b"HTTP/1.0 200 OK\r\n\r\n\
            <html>\
                <body>\
                    <h1>Hello SlugSecurity!</h1>\
                    <img src=\"data:image/jpeg;base64,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\"/>\
                </body>\
            </html>\r\n\
            ",
            )
            .await;
        if let Err(e) = r {
            println!("write error: {:?}", e);
        }

        let r = socket.flush().await;
        if let Err(e) = r {
            println!("flush error: {:?}", e);
        }
        Timer::after(Duration::from_millis(1000)).await;

        socket.close();
        Timer::after(Duration::from_millis(1000)).await;

        socket.abort();
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task");
    println!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        match esp_wifi::wifi::get_wifi_state() {
            WifiState::ApStarted => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::ApStop).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::AccessPoint(AccessPointConfiguration {
                ssid: "esp-wifi".try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            println!("Starting wifi");
            controller.start().await.unwrap();
            println!("Wifi started!");
        }
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiApDevice>>) {
    stack.run().await
}
