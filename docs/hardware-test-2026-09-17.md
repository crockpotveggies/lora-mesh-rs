# First physical LoStik test — 2026-09-17

One user-provided LoStik, antenna attached, testing in Canada. The macOS USB
adapter identified as VID 1a86/PID 7523 (USB2.0-Serial). Firmware readback:
`RN2903 1.0.5 Nov 06 2018 10:45:27`.

The original `/dev/cu.usbserial-10` connection timed out and disappeared from USB
enumeration without the user intentionally unplugging it. Moving to another port
produced `/dev/cu.usbserial-110`, where the production serial worker successfully
initialized, read back the radio profile and entered continuous receive mode.

Two short test transmissions completed through the production controller:
915 MHz, SF7/BW125 kHz, CR4/5, CRC on, transmit power 2 dBm. Both produced a
`Transmitted(1)` event and returned to receive mode. This proves the firmware
reported TX completion; it does not prove another receiver decoded the packet.
No firmware update or EEPROM-save command was issued.

Subsequent reopen attempts produced device I/O errors and the second USB path
also disappeared. Four subsequent receive-only open/initialize cycles passed,
but after another successful probe transmission the secure daemon failed to open
the vanished device. Root cause is not established; the secure-daemon test could
not complete. No over-the-air IPv4 delivery, RF range, RSSI/SNR, or application
throughput measurement is claimed. A stable USB connection and a second radio are
needed for the next acceptance stage.

Raw session evidence is under `target/hardware/` (ignored local artifacts):
`identity-profile.json`, `receive-test-reconnected.log`, `transmit-test.log`,
`transmit-repeat.log`, `single-radio-test.log`, and `summary.json`.

The probe now accepts `--frequency HZ` and `--power DBM`, and prints initialization
commands on readiness failures. A repeatable single-radio test was added:

```sh
cargo build --release --locked --bin loramesh-radio --bin loramesh-mesh --bin loramesh-keygen
python3 scripts/test-hardware-single.py /dev/cu.YOUR_RADIO --frequency 915000000 --power 2 --transmit
```

This explicitly transmits and requires an attached antenna and appropriate local
radio settings. It checks driver TX completion, secure control-frame transmission,
and bounded expiry of IPv4 traffic with an absent peer. Temporary test keys are
removed afterward. It is not an end-to-end delivery test.

## Compatibility follow-up

The [driver compatibility correction](radio-compatibility.md) restores the
settling sequence and MAC reset, replaces macOS serial configuration with one
standard-baud termios update, retains startup retries, and tolerates one lingering
receive error before the TX acknowledgment. The first partial correction still
showed disconnects; after the final port configuration change, four repeated
receive-only opens passed and the previously failing single-radio secure-daemon
acceptance test completed with zero radio failures. The stick remained present.
See the linked report for test evidence and the remaining one-radio limitations.
