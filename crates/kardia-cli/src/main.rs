mod ecg_analysis;
mod ecg_model;
mod ecg_report;
mod live_view;
mod raw_commands;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use kardia_ble::{CaptureOptions, EcgMode};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "kardia")]
#[command(about = "Kardia 6L ECG capture and study workbench")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the current workspace capabilities.
    Doctor,
    /// Describe the planned BLE reverse-engineering capture flow.
    CapturePlan,
    /// Scan for BLE peripherals and mark Kardia 6L candidates.
    Scan {
        /// Scan duration in seconds.
        #[arg(short, long, default_value_t = 10)]
        seconds: u64,
        /// Print every discovered peripheral, not only Kardia-like hits.
        #[arg(long)]
        all: bool,
    },
    /// Connect to a peripheral and print its GATT services.
    GattDump {
        /// Address, platform peripheral id substring, name substring, or service UUID.
        target: Option<String>,
        /// Seconds to scan first so the adapter learns about the device.
        #[arg(short, long, default_value_t = 8)]
        rescan: u64,
        /// Print connection progress to stderr.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Connect to the first Kardia candidate and read all readable characteristics.
    ReadProbe {
        /// Seconds to scan first so the adapter learns about the device.
        #[arg(short, long, default_value_t = 30)]
        rescan: u64,
        /// Print connection progress to stderr.
        #[arg(short, long, default_value_t = true)]
        verbose: bool,
    },
    /// Subscribe to the standard battery notify, then the ECG notify, to
    /// isolate whether the CCCD-write hang is btleplug or device bonding.
    NotifyProbe {
        /// Seconds to scan first so the adapter learns about the device.
        #[arg(short, long, default_value_t = 30)]
        rescan: u64,
        /// Print connection progress to stderr.
        #[arg(short, long, default_value_t = true)]
        verbose: bool,
    },
    /// Connect and save raw command/ECG notifications to CSV-like text.
    CaptureRaw {
        /// Address, platform peripheral id substring, name substring, or service UUID.
        target: Option<String>,
        /// Output path for raw notification records.
        #[arg(short, long)]
        out: PathBuf,
        /// Capture duration in seconds.
        #[arg(short, long, default_value_t = 30)]
        seconds: u64,
        /// Seconds to scan first so the adapter learns about the device.
        #[arg(short, long, default_value_t = 8)]
        rescan: u64,
        /// ECG mode: m1, m2, m3, or m4.
        #[arg(long, default_value = "m2")]
        mode: CliEcgMode,
        /// Print connection and stream progress to stderr.
        #[arg(short, long, default_value_t = true)]
        verbose: bool,
        /// Skip subscribing to the command indication characteristic before unlock write.
        #[arg(long)]
        no_command_indications: bool,
        /// Only subscribe to ECG notifications; skip command indications and unlock write.
        #[arg(long)]
        listen_only: bool,
        /// Show a throttled, labeled six-lead view while preserving every raw packet.
        #[arg(long)]
        live: bool,
    },
    /// Inspect requested mode metadata and observed raw transport properties.
    InspectRaw {
        /// Raw capture produced by `capture-raw`.
        input: PathBuf,
    },
    /// Decode a confirmed M2 raw capture and export six limb leads.
    ExportSixLeadM2 {
        /// Raw capture produced by `capture-raw`.
        input: PathBuf,
        /// Destination CSV path.
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Render a confirmed M2 capture as a standard-grid six-lead ECG report.
    RenderEcg {
        /// Raw capture produced by `capture-raw`.
        input: PathBuf,
        /// Destination `.pdf` or `.svg` path.
        #[arg(short, long)]
        out: PathBuf,
        /// Replace the input path printed in the report header.
        #[arg(long)]
        source_label: Option<String>,
        /// JSON manifest for an optional research-only ONNX classifier.
        #[arg(long)]
        model: Option<PathBuf>,
        /// Seconds into the recording at which the report begins.
        #[arg(long, default_value_t = 0.0)]
        start_seconds: f64,
        /// Seconds to draw (maximum 10 at the standard paper speed).
        #[arg(long, default_value_t = 10.0)]
        seconds: f64,
        /// Horizontal paper speed in millimeters per second.
        #[arg(long, default_value_t = 25.0)]
        speed_mm_s: f64,
        /// Vertical gain in millimeters per millivolt.
        #[arg(long, default_value_t = 10.0)]
        gain_mm_mv: f64,
        /// Provisional millivolts represented by one stored raw count.
        #[arg(long, default_value_t = 0.000_152_587_890_625)]
        mv_per_count: f64,
        /// Reverse the device-native signal polarity.
        #[arg(long)]
        invert: bool,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum CliEcgMode {
    M1,
    M2,
    M3,
    M4,
}

impl From<CliEcgMode> for EcgMode {
    fn from(value: CliEcgMode) -> Self {
        match value {
            CliEcgMode::M1 => Self::SingleLead300Hz,
            CliEcgMode::M2 => Self::DualLead300Hz,
            CliEcgMode::M3 => Self::SingleLead600Hz,
            CliEcgMode::M4 => Self::DualLead600Hz,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Doctor => doctor(),
        Command::CapturePlan => capture_plan(),
        Command::Scan { seconds, all } => scan(seconds, all).await,
        Command::GattDump {
            target,
            rescan,
            verbose,
        } => gatt_dump(&target, rescan, verbose).await,
        Command::ReadProbe { rescan, verbose } => read_probe(rescan, verbose).await,
        Command::NotifyProbe { rescan, verbose } => notify_probe(rescan, verbose).await,
        Command::CaptureRaw {
            target,
            out,
            seconds,
            rescan,
            mode,
            verbose,
            no_command_indications,
            listen_only,
            live,
        } => {
            capture_raw(
                CaptureOptions {
                    target: target.unwrap_or_default(),
                    out,
                    seconds,
                    rescan_seconds: rescan,
                    mode: mode.into(),
                    verbose,
                    command_indications: !no_command_indications && !listen_only,
                    unlock_write: !listen_only,
                },
                live,
            )
            .await
        }
        Command::InspectRaw { input } => raw_commands::inspect_raw(&input),
        Command::ExportSixLeadM2 { input, out } => raw_commands::export_six_lead_m2(&input, &out),
        Command::RenderEcg {
            input,
            out,
            source_label,
            model,
            start_seconds,
            seconds,
            speed_mm_s,
            gain_mm_mv,
            mv_per_count,
            invert,
        } => ecg_report::render(
            &input,
            &out,
            ecg_report::ReportOptions {
                start_seconds,
                duration_seconds: seconds,
                speed_mm_s,
                gain_mm_mv,
                mv_per_count,
                invert,
            },
            source_label.as_deref(),
            model.as_deref(),
        ),
    }
}

fn doctor() -> Result<()> {
    println!("kardia-rs workspace");
    println!("core: ECG sample and six-lead derivation types available");
    println!("ble: live scan, GATT inspection, journaled raw capture, and M2 decoding available");
    println!(
        "cli: live M2 view, raw inspection, six-lead CSV, and vector ECG reports with optional research-only ONNX rhythm similarity available"
    );
    println!(
        "limits: report scale is provisional; signal polarity, physical calibration, and exposed M4 600 Hz remain unverified"
    );
    Ok(())
}

fn capture_plan() -> Result<()> {
    println!("1. scan without filters; keep advertisements exposing AC060001-328C-A28F-9846-5A8AA212661B");
    println!("2. connect, bond if required, and discover GATT services");
    println!(
        "3. resolve the 6L service plus command AC060002-... and ECG AC060003-... characteristics"
    );
    println!("4. enable indications on the command characteristic");
    println!("5. write the mode/unlock command, e.g. M2 K<sha256(Triangle + device_name)[0..16]>");
    println!("6. enable notifications on the ECG characteristic and persist every raw payload with timestamps");
    println!("7. inspect requested metadata separately from observed packet shape and cadence");
    println!("8. decode only from replayable raw fixtures after packet structure is confirmed");
    Ok(())
}

async fn scan(seconds: u64, all: bool) -> Result<()> {
    let devices = kardia_ble::scan(Duration::from_secs(seconds)).await?;
    let mut printed = 0usize;
    for device in devices {
        if !all && !device.is_kardia_6l {
            continue;
        }
        printed += 1;
        let marker = if device.is_kardia_6l {
            "KARDIA_6L"
        } else {
            "other"
        };
        println!(
            "[{marker}] id={:?} address={} rssi={:?} name={:?} services={:?}",
            device.id,
            device.address,
            device.rssi,
            device.fingerprint.name,
            device.fingerprint.advertised_services
        );
    }
    if printed == 0 {
        if all {
            println!("no BLE peripherals observed in {seconds}s");
        } else {
            println!("no Kardia 6L candidates observed in {seconds}s; try `kardia scan --all`");
        }
    }
    Ok(())
}

async fn gatt_dump(target: &Option<String>, rescan: u64, verbose: bool) -> Result<()> {
    let dump = match target {
        Some(target) => kardia_ble::live::gatt_dump(target, Duration::from_secs(rescan)).await?,
        None => {
            kardia_ble::live::gatt_dump_first_kardia(Duration::from_secs(rescan), verbose).await?
        }
    };
    print!("{dump}");
    Ok(())
}

async fn read_probe(rescan: u64, verbose: bool) -> Result<()> {
    let reads =
        kardia_ble::live::read_probe_first_kardia(Duration::from_secs(rescan), verbose).await?;
    print!("{reads}");
    Ok(())
}

async fn notify_probe(rescan: u64, verbose: bool) -> Result<()> {
    let report =
        kardia_ble::live::notify_probe_first_kardia(Duration::from_secs(rescan), verbose).await?;
    print!("{report}");
    Ok(())
}

async fn capture_raw(options: CaptureOptions, live: bool) -> Result<()> {
    let out = options.out.clone();
    let seconds = options.seconds;
    let stats = if live {
        if options.mode != EcgMode::DualLead300Hz {
            return Err(anyhow!(
                "--live currently requires --mode m2; decoding for {} is not confirmed",
                options.mode.setting()
            ));
        }
        println!(
            "live labels: Ch1/I and Ch2/II confirmed by electrode-removal tests; polarity and scale remain uncalibrated"
        );
        let mut frame_count = 0u64;
        let result = kardia_ble::capture_raw_with_m2_observer(options, move |_, frame| {
            frame_count += 1;
            if frame_count == 1 || frame_count % 4 == 0 {
                print!("\r\x1b[2K{}", live_view::format_m2(frame_count, frame));
                std::io::stdout().flush().ok();
            }
        })
        .await;
        println!();
        result?
    } else if options.target.is_empty() {
        kardia_ble::live::capture_raw_first_kardia(options).await?
    } else {
        kardia_ble::live::capture_raw(options).await?
    };
    println!(
        "captured {} ECG packets ({} bytes) and {} command indications over {}s to {}",
        stats.ecg_packets,
        stats.ecg_bytes,
        stats.command_packets,
        seconds,
        out.display()
    );
    raw_commands::inspect_raw(&out)?;
    Ok(())
}
