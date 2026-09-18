# Independent command fixtures

`rn2903.json` is a hand-authored conformance sequence, not output recorded from the
virtual device or the controller. Command/reply behavior follows the Microchip
RN2903 command reference (DS40001811A, sections 2.3.2, 2.4.6, 2.5.1 and 2.5.2):
https://ww1.microchip.com/downloads/en/DeviceDoc/40001811A.pdf

The version suffix `virtual` identifies the emulator. `radio rxstop` is the LoStik
firmware extension already used by this project; verify support on physical devices.
No hardware captures are available yet. These fixtures do not certify firmware
compatibility. Delayed completion, watchdog and interleaved RX events are covered
by separate controller/medium tests with explicitly supplied responses.
