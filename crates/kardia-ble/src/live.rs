use crate::device::DeviceFingerprint;
use crate::kardia6l::{
    command_for_mode, uuid_matches, EcgMode, BATTERY_LEVEL_CHARACTERISTIC_UUID,
    KARDIA_6L_SIX_LEAD_CMD_CHARACTERISTIC_UUID, KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID,
};
use anyhow::{anyhow, Context, Result};
use btleplug::api::{
    BDAddr, Central, CentralEvent, CharPropFlags, Characteristic, Manager as _, Peripheral as _,
    ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use futures::StreamExt;
use std::fmt::Write as FmtWrite;
use std::io::Write as IoWrite;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct DiscoveredDevice {
    pub id: PeripheralId,
    pub address: BDAddr,
    pub fingerprint: DeviceFingerprint,
    pub rssi: Option<i16>,
    pub is_kardia_6l: bool,
}

#[derive(Debug, Clone)]
pub struct CaptureOptions {
    pub target: String,
    pub out: std::path::PathBuf,
    pub seconds: u64,
    pub rescan_seconds: u64,
    pub mode: EcgMode,
    pub verbose: bool,
    pub command_indications: bool,
    pub unlock_write: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawCaptureStats {
    pub ecg_packets: usize,
    pub ecg_bytes: usize,
    pub command_packets: usize,
}

pub async fn scan(duration: Duration) -> Result<Vec<DiscoveredDevice>> {
    let mgr = manager().await?;
    let adapter = default_adapter(&mgr).await?;
    scan_with_adapter(&adapter, duration).await
}

pub async fn gatt_dump(target: &str, rescan: Duration) -> Result<String> {
    let mgr = manager().await?;
    let adapter = default_adapter(&mgr).await?;
    scan_with_adapter(&adapter, rescan).await?;
    let peripheral = resolve_peripheral(&adapter, target).await?;
    dump_connected_gatt(&peripheral, false).await
}

pub async fn gatt_dump_first_kardia(rescan: Duration, verbose: bool) -> Result<String> {
    let mgr = manager().await?;
    let adapter = default_adapter(&mgr).await?;
    let peripheral = scan_first_kardia_peripheral(&adapter, rescan, verbose).await?;
    dump_connected_gatt(&peripheral, verbose).await
}

pub async fn read_probe_first_kardia(rescan: Duration, verbose: bool) -> Result<String> {
    let mgr = manager().await?;
    let adapter = default_adapter(&mgr).await?;
    let peripheral = scan_first_kardia_peripheral(&adapter, rescan, verbose).await?;
    connect_and_discover(&peripheral, verbose).await?;

    let mut out = String::new();
    for service in peripheral.services() {
        for ch in service.characteristics {
            if !ch.properties.contains(CharPropFlags::READ) {
                continue;
            }
            let value = ble_timeout(
                "read characteristic",
                peripheral.read(&ch),
                Duration::from_secs(8),
            )
            .await
            .with_context(|| format!("read {}", ch.uuid))?;
            out.push_str(&format!("{} = {}\n", ch.uuid, printable_or_hex(&value)));
        }
    }

    peripheral.disconnect().await.ok();
    Ok(out)
}

pub async fn capture_raw(options: CaptureOptions) -> Result<RawCaptureStats> {
    let mgr = manager().await?;
    let adapter = default_adapter(&mgr).await?;
    scan_with_adapter(&adapter, Duration::from_secs(options.rescan_seconds)).await?;
    let peripheral = resolve_peripheral(&adapter, &options.target).await?;
    capture_raw_from_peripheral(peripheral, options).await
}

pub async fn capture_raw_first_kardia(mut options: CaptureOptions) -> Result<RawCaptureStats> {
    let mgr = manager().await?;
    let adapter = default_adapter(&mgr).await?;
    let peripheral = scan_first_kardia_peripheral(
        &adapter,
        Duration::from_secs(options.rescan_seconds),
        options.verbose,
    )
    .await?;
    if let Some(props) = peripheral.properties().await? {
        if let Some(name) = props.local_name {
            options.target = name;
        }
    }
    capture_raw_from_peripheral(peripheral, options).await
}

async fn capture_raw_from_peripheral(
    peripheral: Peripheral,
    options: CaptureOptions,
) -> Result<RawCaptureStats> {
    connect_and_discover(&peripheral, options.verbose).await?;

    let props = peripheral
        .properties()
        .await?
        .ok_or_else(|| anyhow!("connected peripheral has no properties"))?;
    let device_name = props
        .local_name
        .as_deref()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| anyhow!("device has no advertised local name; cannot build unlock token"))?;

    let cmd_uuid = parse_uuid(KARDIA_6L_SIX_LEAD_CMD_CHARACTERISTIC_UUID)?;
    let ecg_uuid = parse_uuid(KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID)?;
    let cmd_char = find_characteristic(&peripheral, cmd_uuid)?;
    let ecg_char = find_characteristic(&peripheral, ecg_uuid)?;
    let command = command_for_mode(device_name, options.mode);

    if options.command_indications {
        if options.verbose {
            eprintln!("subscribing to command indications on {cmd_uuid}");
        }
        ble_timeout(
            "enable command indications",
            peripheral.subscribe(&cmd_char),
            Duration::from_secs(45),
        )
        .await?;
    } else if options.verbose {
        eprintln!("skipping command indications on {cmd_uuid}");
    }
    if options.unlock_write {
        if options.verbose {
            eprintln!("writing unlock/mode command {command:?}");
        }
        ble_timeout(
            "write unlock command",
            peripheral.write(&cmd_char, command.as_bytes(), WriteType::WithResponse),
            Duration::from_secs(45),
        )
        .await
        .with_context(|| format!("write unlock command {command:?}"))?;
    } else if options.verbose {
        eprintln!("skipping unlock/mode command {command:?}");
    }
    if options.verbose {
        eprintln!("subscribing to ECG notifications on {ecg_uuid}");
    }
    ble_timeout(
        "enable ECG notifications",
        peripheral.subscribe(&ecg_char),
        Duration::from_secs(45),
    )
    .await?;

    let mut notifications = peripheral.notifications().await?;
    let mut writer = open_capture_file(&options.out)?;
    if options.verbose {
        eprintln!(
            "recording {}s of raw notifications to {}",
            options.seconds,
            options.out.display()
        );
    }
    writeln!(
        writer,
        "# unix_micros,characteristic_uuid,payload_hex mode={} sample_rate_hz={} leads={} device_name={:?}",
        options.mode.setting(),
        options.mode.sample_rate_hz(),
        options.mode.lead_count(),
        device_name
    )?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(options.seconds);
    let mut stats = RawCaptureStats {
        ecg_packets: 0,
        ecg_bytes: 0,
        command_packets: 0,
    };

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, notifications.next()).await {
            Ok(Some(notification)) => {
                let now = unix_micros(SystemTime::now())?;
                writeln!(
                    writer,
                    "{now},{},{}",
                    notification.uuid,
                    hex(&notification.value)
                )?;
                if notification.uuid == ecg_uuid {
                    stats.ecg_packets += 1;
                    stats.ecg_bytes += notification.value.len();
                } else if notification.uuid == cmd_uuid {
                    stats.command_packets += 1;
                }
            }
            Ok(None) | Err(_) => break,
        }
    }

    writer.flush().ok();
    peripheral.disconnect().await.ok();
    Ok(stats)
}

/// Disambiguate the CCCD-write hang: subscribe to the standard battery-level
/// characteristic (unencrypted notify) and then the Kardia ECG characteristic.
///
/// - battery hangs too  => btleplug/CoreBluetooth subscribe is broken here.
/// - battery ok, ECG hangs => the Kardia CCCD needs bonding/encryption.
pub async fn notify_probe_first_kardia(rescan: Duration, verbose: bool) -> Result<String> {
    let mgr = manager().await?;
    let adapter = default_adapter(&mgr).await?;
    let peripheral = scan_first_kardia_peripheral(&adapter, rescan, verbose).await?;
    connect_and_discover(&peripheral, verbose).await?;

    let battery_uuid = parse_uuid(BATTERY_LEVEL_CHARACTERISTIC_UUID)?;
    let ecg_uuid = parse_uuid(KARDIA_6L_SIX_LEAD_ECG_CHARACTERISTIC_UUID)?;

    let mut out = String::new();
    for (label, uuid) in [("battery-level", battery_uuid), ("ecg-stream", ecg_uuid)] {
        let ch = match find_characteristic(&peripheral, uuid) {
            Ok(ch) => ch,
            Err(err) => {
                let _ = writeln!(out, "{label} ({uuid}): characteristic not found: {err}");
                continue;
            }
        };
        if verbose {
            eprintln!("subscribing to {label} notify on {uuid}");
        }
        match ble_timeout(
            "subscribe notify",
            peripheral.subscribe(&ch),
            Duration::from_secs(15),
        )
        .await
        {
            Ok(()) => {
                let _ = writeln!(out, "{label} ({uuid}): subscribe OK");
                peripheral.unsubscribe(&ch).await.ok();
            }
            Err(err) => {
                let _ = writeln!(out, "{label} ({uuid}): subscribe FAILED: {err:#}");
            }
        }
    }

    peripheral.disconnect().await.ok();
    Ok(out)
}

async fn manager() -> Result<Manager> {
    Manager::new().await.context("init btleplug Manager")
}

async fn default_adapter(mgr: &Manager) -> Result<Adapter> {
    let adapters = mgr.adapters().await.context("list BLE adapters")?;
    adapters
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no BLE adapter found"))
}

async fn scan_with_adapter(adapter: &Adapter, duration: Duration) -> Result<Vec<DiscoveredDevice>> {
    let mut events = adapter.events().await?;
    adapter
        .start_scan(ScanFilter::default())
        .await
        .context("start BLE scan")?;

    let deadline = tokio::time::Instant::now() + duration;
    let mut hits: Vec<DiscoveredDevice> = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, events.next()).await {
            Ok(Some(evt)) => {
                if let Some(device) = consider(adapter, evt).await? {
                    if !hits.iter().any(|hit| hit.id == device.id) {
                        hits.push(device);
                    }
                }
            }
            Ok(None) | Err(_) => break,
        }
    }

    adapter.stop_scan().await.ok();
    Ok(hits)
}

async fn scan_first_kardia_peripheral(
    adapter: &Adapter,
    duration: Duration,
    verbose: bool,
) -> Result<Peripheral> {
    if verbose {
        eprintln!("scanning for Kardia 6L candidate for {duration:?}");
    }
    let mut events = adapter.events().await?;
    adapter
        .start_scan(ScanFilter::default())
        .await
        .context("start BLE scan")?;

    let deadline = tokio::time::Instant::now() + duration;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            adapter.stop_scan().await.ok();
            return Err(anyhow!("no Kardia 6L candidate observed in {:?}", duration));
        }

        match tokio::time::timeout(remaining, events.next()).await {
            Ok(Some(evt)) => {
                let Some(device) = consider(adapter, evt).await? else {
                    continue;
                };
                if !device.is_kardia_6l {
                    continue;
                }
                if verbose {
                    eprintln!(
                        "found Kardia candidate id={:?} name={:?} rssi={:?} services={:?}",
                        device.id,
                        device.fingerprint.name,
                        device.rssi,
                        device.fingerprint.advertised_services
                    );
                }
                adapter.stop_scan().await.ok();
                return adapter
                    .peripheral(&device.id)
                    .await
                    .context("resolve discovered Kardia peripheral");
            }
            Ok(None) | Err(_) => {
                adapter.stop_scan().await.ok();
                return Err(anyhow!("no Kardia 6L candidate observed in {:?}", duration));
            }
        }
    }
}

async fn consider(adapter: &Adapter, evt: CentralEvent) -> Result<Option<DiscoveredDevice>> {
    let id = match evt {
        CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) => id,
        _ => return Ok(None),
    };
    let peripheral = adapter.peripheral(&id).await?;
    let props = match peripheral.properties().await? {
        Some(props) => props,
        None => return Ok(None),
    };
    let fingerprint = DeviceFingerprint {
        name: props.local_name,
        address: Some(props.address.to_string()),
        advertised_services: props.services.iter().map(ToString::to_string).collect(),
        manufacturer_data: props
            .manufacturer_data
            .iter()
            .map(|(id, data)| (*id, data.clone()))
            .collect(),
    };
    let is_kardia_6l = fingerprint.looks_like_kardia_6l();

    Ok(Some(DiscoveredDevice {
        id,
        address: props.address,
        fingerprint,
        rssi: props.rssi,
        is_kardia_6l,
    }))
}

async fn resolve_peripheral(adapter: &Adapter, target: &str) -> Result<Peripheral> {
    let needle = target.to_ascii_lowercase();
    for peripheral in adapter.peripherals().await? {
        let addr_match = peripheral
            .address()
            .to_string()
            .eq_ignore_ascii_case(target);
        let id_str = format!("{:?}", peripheral.id());
        let id_match = id_str.to_ascii_lowercase().contains(&needle);
        let props = peripheral.properties().await?;
        let name_match = props
            .as_ref()
            .and_then(|props| props.local_name.as_deref())
            .map(|name| name.to_ascii_lowercase().contains(&needle))
            .unwrap_or(false);
        let service_match = props
            .as_ref()
            .map(|props| {
                props
                    .services
                    .iter()
                    .any(|uuid| uuid_matches(&uuid.to_string(), target))
            })
            .unwrap_or(false);

        if addr_match || id_match || name_match || service_match {
            return Ok(peripheral);
        }
    }
    Err(anyhow!("no peripheral matched target {target:?}"))
}

async fn connect_and_discover(peripheral: &Peripheral, verbose: bool) -> Result<()> {
    if !peripheral.is_connected().await? {
        if verbose {
            eprintln!("connecting to peripheral id={:?}", peripheral.id());
        }
        ble_timeout("connect", peripheral.connect(), Duration::from_secs(12)).await?;
        sleep(Duration::from_millis(250)).await;
    }
    if verbose {
        eprintln!("discovering GATT services");
    }
    ble_timeout(
        "discover services",
        peripheral.discover_services(),
        Duration::from_secs(12),
    )
    .await?;
    if verbose {
        eprintln!("discovered {} services", peripheral.services().len());
    }
    Ok(())
}

async fn dump_connected_gatt(peripheral: &Peripheral, verbose: bool) -> Result<String> {
    connect_and_discover(peripheral, verbose).await?;

    let mut out = String::new();
    for service in peripheral.services() {
        out.push_str(&format!(
            "service {} primary={}\n",
            service.uuid, service.primary
        ));
        for ch in service.characteristics {
            out.push_str(&format!("  char {} props={:?}\n", ch.uuid, ch.properties));
            for desc in ch.descriptors {
                out.push_str(&format!("    desc {}\n", desc.uuid));
            }
        }
    }

    peripheral.disconnect().await.ok();
    Ok(out)
}

fn find_characteristic(peripheral: &Peripheral, uuid: Uuid) -> Result<Characteristic> {
    for service in peripheral.services() {
        for ch in service.characteristics {
            if ch.uuid == uuid {
                return Ok(ch);
            }
        }
    }
    Err(anyhow!("characteristic {uuid} not found"))
}

fn parse_uuid(value: &str) -> Result<Uuid> {
    Uuid::parse_str(value).with_context(|| format!("parse UUID {value}"))
}

fn open_capture_file(path: &Path) -> Result<std::io::BufWriter<std::fs::File>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create output directory {}", parent.display()))?;
    }
    Ok(std::io::BufWriter::new(
        std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?,
    ))
}

fn unix_micros(time: SystemTime) -> Result<u128> {
    Ok(time
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_micros())
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}

fn printable_or_hex(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text)
            if text
                .chars()
                .all(|ch| !ch.is_control() || ch.is_ascii_whitespace()) =>
        {
            format!("{text:?}")
        }
        _ => format!("0x{}", hex(bytes)),
    }
}

pub fn characteristic_supports_notify_or_indicate(ch: &Characteristic) -> bool {
    ch.properties
        .intersects(CharPropFlags::NOTIFY | CharPropFlags::INDICATE)
}

async fn ble_timeout<F, T>(stage: &'static str, future: F, timeout_after: Duration) -> Result<T>
where
    F: std::future::Future<Output = Result<T, btleplug::Error>>,
{
    tokio::time::timeout(timeout_after, future)
        .await
        .with_context(|| format!("{stage} timed out after {timeout_after:?}"))?
        .with_context(|| stage)
}
