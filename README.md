# RAM Buddy

RAM Buddy is a small Windows notification-area app. Open `ram-buddy.exe` and look for the animated buddy under the **^** beside the clock. Hover to see physical RAM usage.

| RAM used | Animation |
| --- | --- |
| Below 70% | Green buddy runs |
| 70–84% | Amber buddy strains and sweats |
| 85% or higher | Pink buddy panics |

Click the buddy to review Node.js and Python processes using at least **256 MiB of resident RAM each**. Confirm to force-stop the matching processes in your Windows account and session. This can discard unsaved work inside those processes. Right-click the icon to exit.

Build from source with `cargo build --release`. The executable is at `target/release/ram-buddy.exe`.
