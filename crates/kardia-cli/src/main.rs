use anyhow::Result;
use clap::{Parser, Subcommand};
use kardia_ble::{CaptureOptions, EcgMode};
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
        } => {
            capture_raw(CaptureOptions {
                target: target.unwrap_or_default(),
                out,
                seconds,
                rescan_seconds: rescan,
                mode: mode.into(),
                verbose,
                command_indications: !no_command_indications && !listen_only,
                unlock_write: !listen_only,
            })
            .await
        }
    }
}

fn doctor() -> Result<()> {
    println!("kardia-rs workspace");
    println!("core: ECG sample and six-lead derivation types available");
    println!("ble: live scan, GATT dump, and raw capture paths available through btleplug");
    println!("cli: operational");
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
    println!("7. decode only from replayable raw fixtures after packet structure is confirmed");
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

async fn capture_raw(options: CaptureOptions) -> Result<()> {
    let out = options.out.clone();
    let seconds = options.seconds;
    let stats = if options.target.is_empty() {
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
    Ok(())
}
