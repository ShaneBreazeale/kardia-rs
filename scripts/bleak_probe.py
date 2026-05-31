#!/usr/bin/env python3
"""Kardia 6L BLE diagnostic via bleak (reference CoreBluetooth stack).

Purpose: decide whether the CCCD-write / write-with-response hang seen from the
Rust/btleplug path is a btleplug bug or a device bonding/security requirement.
bleak talks to the same CoreBluetooth backend Apple ships, so if a step works
here but hangs in btleplug it is a library issue; if it hangs here too it is the
device or macOS pairing.

Usage:
    pip install bleak
    python3 scripts/bleak_probe.py                 # auto-pick first Kardia
    python3 scripts/bleak_probe.py --address <UUID> # target a known peripheral
    python3 scripts/bleak_probe.py --mode m2 --stream  # also unlock + capture ECG
    python3 scripts/bleak_probe.py --token-name Kardia6L  # override unlock name

Each step is wrapped in its own timeout and the outcome is printed, so a hang on
one step does not hide the others.
"""

import argparse
import asyncio
import hashlib
import sys
import time

from bleak import BleakClient, BleakScanner

SIX_LEAD_SERVICE = "ac060001-328c-a28f-9846-5a8aa212661b"
CMD_CHAR = "ac060002-328c-a28f-9846-5a8aa212661b"  # write + indicate
ECG_CHAR = "ac060003-328c-a28f-9846-5a8aa212661b"  # notify
READ_005 = "ac060005-328c-a28f-9846-5a8aa212661b"
READ_006 = "ac060006-328c-a28f-9846-5a8aa212661b"
BATTERY_CHAR = "00002a19-0000-1000-8000-00805f9b34fb"  # read + notify (control)
MODEL_CHAR = "00002a24-0000-1000-8000-00805f9b34fb"

MODES = {"m1": "M1", "m2": "M2", "m3": "M3", "m4": "M4"}


def unlock_command(device_name: str, mode: str) -> str:
    digest = hashlib.sha256(("Triangle" + device_name).encode()).hexdigest()
    return f"{MODES[mode]} K{digest[:16]}"


def looks_like_kardia(dev, adv) -> bool:
    name = (dev.name or adv.local_name or "").lower()
    if "kardia" in name or "alivecor" in name:
        return True
    uuids = [u.lower() for u in (adv.service_uuids or [])]
    return SIX_LEAD_SERVICE in uuids


async def step(label: str, coro, timeout: float):
    """Run one BLE op under its own timeout; report instead of aborting."""
    t0 = time.monotonic()
    try:
        result = await asyncio.wait_for(coro, timeout=timeout)
        dt = time.monotonic() - t0
        print(f"  [OK   {dt:5.2f}s] {label}")
        return result
    except asyncio.TimeoutError:
        print(f"  [HANG {timeout:5.2f}s] {label}  <-- no CoreBluetooth callback")
        return None
    except Exception as exc:  # noqa: BLE001 - diagnostic surface
        dt = time.monotonic() - t0
        print(f"  [FAIL {dt:5.2f}s] {label}: {exc!r}")
        return None


async def find_device(address: str | None, scan_secs: float):
    if address:
        print(f"scanning for address {address} ({scan_secs:.0f}s)")
        dev = await BleakScanner.find_device_by_address(address, timeout=scan_secs)
        if dev is None:
            print("  not found")
        return dev

    print(f"scanning for first Kardia candidate ({scan_secs:.0f}s)")
    found = {}

    def cb(dev, adv):
        if looks_like_kardia(dev, adv) and dev.address not in found:
            found[dev.address] = (dev, adv)
            print(f"  candidate name={dev.name!r} address={dev.address} rssi={adv.rssi}")

    scanner = BleakScanner(detection_callback=cb)
    await scanner.start()
    deadline = time.monotonic() + scan_secs
    while time.monotonic() < deadline and not found:
        await asyncio.sleep(0.2)
    await scanner.stop()
    if not found:
        print("  no Kardia candidate observed")
        return None
    return next(iter(found.values()))[0]


async def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--address", help="target peripheral address/UUID")
    ap.add_argument("--scan-secs", type=float, default=90.0)
    ap.add_argument("--op-timeout", type=float, default=15.0)
    ap.add_argument("--mode", choices=MODES, default="m2")
    ap.add_argument("--stream", action="store_true", help="also unlock + capture ECG")
    ap.add_argument("--stream-secs", type=float, default=10.0)
    ap.add_argument(
        "--token-name",
        help="override the name fed into the unlock hash (default: advertised name)",
    )
    args = ap.parse_args()

    dev = await find_device(args.address, args.scan_secs)
    if dev is None:
        return 1

    device_name = dev.name or ""
    print(f"\nconnecting to {device_name!r} {dev.address}")
    async with BleakClient(dev) as client:
        print(f"connected={client.is_connected}\n")

        print("reads (baseline, should all be OK):")
        for label, uuid in [
            ("model 2a24", MODEL_CHAR),
            ("ac060005", READ_005),
            ("ac060006", READ_006),
        ]:
            val = await step(f"read {label}", client.read_gatt_char(uuid), args.op_timeout)
            if val is not None:
                print(f"         value=0x{val.hex()} ascii={val!r}")

        # Counters live in closures so the notify handlers can mutate them.
        counts = {"battery": 0, "ecg": 0, "cmd": 0}

        def make_handler(key):
            def handler(_char, data: bytearray):
                counts[key] += 1
                if counts[key] <= 3:
                    print(f"         {key} notify #{counts[key]} 0x{data.hex()}")
            return handler

        print("\nsubscribe control vs target (the decisive split):")
        await step(
            "subscribe battery 2a19 (unencrypted control)",
            client.start_notify(BATTERY_CHAR, make_handler("battery")),
            args.op_timeout,
        )
        await step(
            "subscribe ECG ac060003 (target, notify-only)",
            client.start_notify(ECG_CHAR, make_handler("ecg")),
            args.op_timeout,
        )

        if args.stream:
            token_name = args.token_name if args.token_name is not None else device_name
            command = unlock_command(token_name, args.mode)
            print(f"\nstream attempt (token from name {token_name!r}):")
            print(f"  command = {command!r}")
            await step(
                "enable command indications ac060002",
                client.start_notify(CMD_CHAR, make_handler("cmd")),
                args.op_timeout,
            )
            await step(
                "write unlock command (with response)",
                client.write_gatt_char(CMD_CHAR, command.encode(), response=True),
                args.op_timeout,
            )
            print(f"  collecting ECG for {args.stream_secs:.0f}s ...")
            await asyncio.sleep(args.stream_secs)

        print(f"\ncounts: {counts}")

    print("\ndone")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
