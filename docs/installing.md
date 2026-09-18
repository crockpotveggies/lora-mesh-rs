# Installing LoRa Mesh

Packages contain command-line programs, examples, documentation, license notices
and build/checksum manifests. They do not create identities, configure radios,
install network drivers, change firewall rules or start a service automatically.

## Platform support

| Platform | Installers | Included tools | Native IP networking |
| --- | --- | --- | --- |
| Linux x86_64 / ARM64 | DEB, portable tar.gz | Secure daemon, keygen, radio probe, both simulators | Linux TUN |
| macOS Apple Silicon / Intel | PKG, DMG containing that PKG, portable tar.gz | Same tools as Linux; daemon uses Unix datagrams | Not implemented |
| Windows x86_64 | Per-user Setup EXE, portable ZIP | Radio probe and both deterministic simulators | Not implemented |

The Windows package does **not** contain a placeholder secure daemon or keygen.
The portable protocol, crypto and simulator code compiles on Windows, but durable
security storage and the daemon remain Unix-only. Windows PTY mode is unavailable;
physical COM-port probing works through the serialport adapter. No native Windows
installer has been executed on the development Mac; the Windows CI job builds,
installs and tests it on a native runner.

Choose the artifact whose architecture matches the machine. GitHub builds target
Ubuntu 24.04 (glibc 2.39+) on Linux, macOS 15 on Mac, and the Windows MSVC x86_64
runtime. The DEB declares the build host's glibc requirement; locally built packages
may have a different floor. Windows packages are command-line tools, not a GUI.

## Install

Download from this repository's GitHub Releases and verify the matching `.sha256`
sidecar. The release includes build metadata and a source archive for the exact tag.

**macOS:** open the DMG and run its PKG, or open the standalone PKG. Installation
requires administrator approval. Commands go into `/usr/local/bin`; resources go
into `/usr/local/share/loramesh`. Then, from Terminal:

```sh
loramesh-radio --version
loramesh-mesh-sim /usr/local/share/loramesh/scenarios/mesh/line.json --output /tmp/mesh.json
```

**Linux:** use `sudo apt install ./loramesh-VERSION-TARGET.deb`. Commands go into
`/usr/bin`; resources into `/usr/share/loramesh`. Dependencies include libudev,
libgcc and iproute2. For actual IP operation, provision/configure a service account,
radio and keys as described in [secure operation](secure-mesh.md).

**Windows:** run the `-setup.exe` installer. It requires no administrator rights and
installs under `%LOCALAPPDATA%\Programs\LoRaMesh`. Open **LoRa Mesh Command Prompt**
from the Start menu; the prompt starts in the installed `bin` directory. PATH is
not changed. For example:

```bat
loramesh-radio.exe --help
loramesh-mesh-sim.exe ..\scenarios\mesh\line.json --output "%TEMP%\mesh.json"
```

For portable archives, extract the directory and run commands from its `bin`
folder; resources are adjacent under `scenarios` and `docs`.

## Upgrade and uninstall

Install a newer package over the existing installation. Keep all deployment keys,
state and editable configuration outside the installation directory. Packages never
provision, migrate, reset or remove external replay state. In particular, never
restore an older state snapshot using the same keys.

- Windows: use Installed Apps → LoRa Mesh Tools → Uninstall, or its Start menu shortcut.
- Linux: `sudo apt remove loramesh`.
- macOS: `sudo sh /usr/local/share/loramesh/uninstall.sh` removes packaged commands,
  resources and its receipt, leaving external configuration/state alone.
- Portable archives: remove the extracted directory after moving any user-created
  data elsewhere. Keep deployment identities outside that directory from the start.

## Trust and signing

Unsigned builds are supported and their metadata says so. They can trigger
Gatekeeper/SmartScreen prompts; they are not represented as trusted/notarized
releases. The macOS release workflow can sign and notarize PKGs/DMGs when publisher
credentials are configured. Windows installers currently remain unsigned. Do not
change system-wide security settings to make an installer run.

See [building and publishing releases](releasing.md) for exact workflow behavior,
signing configuration, version/tag requirements and local test commands.
