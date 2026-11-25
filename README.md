# RAM Buddy

RAM Buddy is a small Windows tray app that shows how much physical RAM is in use. It also lets you review and force-stop memory-heavy Node.js and Python processes from the same icon.

## Download and run

1. Download [RAM Buddy for Windows x64](https://github.com/sanketpatel32/ram-buddy/releases/latest/download/ram-buddy-windows-x64.zip) from GitHub and extract the ZIP.
2. Run `ram-buddy.exe`. The app is portable and runs as a regular Windows user. The executable is unsigned.
3. Look for the buddy beside the clock or under the **^** arrow for hidden icons. Hover to see the RAM percentage, used and total GiB, and the cleanup threshold.

Each [GitHub release](https://github.com/sanketpatel32/ram-buddy/releases) includes a SHA-256 checksum for the ZIP. To verify your download, run `Get-FileHash .\ram-buddy-windows-x64.zip -Algorithm SHA256` in PowerShell and compare the hash with `SHA256SUMS.txt`.

### Upgrade an existing copy

1. Right-click the tray icon and choose **Exit RAM Buddy**.
2. Download and extract the latest ZIP. Replace your old `ram-buddy.exe` with the new one, then run it.
3. If you already saved a cleanup threshold, RAM Buddy remembers it after the upgrade.

## What the buddy does

| Physical RAM in use | Animation |
| --- | --- |
| Below 70% | Green buddy runs |
| 70–84% | Amber buddy strains and sweats |
| 85% or higher | Pink buddy panics |

- **Click the icon** to preview qualifying Node.js and Python processes in your account and Windows session. Choose **Yes** to force-stop the matching group or **No** to cancel. Force-stopping a process can discard its unsaved work.
- **Right-click the icon** to review Node.js or Python separately, change the minimum RAM per process, or exit. The threshold defaults to **256 MiB of resident RAM** and can be set to **128, 256, 512, or 1024 MiB**. RAM Buddy remembers your choice when restarted.

The app reads total RAM use periodically and scans processes when you open the cleanup preview. It runs locally and sends no telemetry.

## Build from source

Install stable Rust with the Windows MSVC toolchain and the Visual Studio Build Tools with the C++ desktop workload. In PowerShell, run:

```powershell
git clone https://github.com/sanketpatel32/ram-buddy.git
cd ram-buddy
cargo build --release --locked
.\target\release\ram-buddy.exe
```

## Contribute

See the [contributor guide](CONTRIBUTING.md) for local checks and pull requests. Report bugs or suggest improvements through [GitHub Issues](https://github.com/sanketpatel32/ram-buddy/issues).

RAM Buddy is released under the [MIT License](LICENSE).
