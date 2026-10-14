# Toolchain

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc | mise | Build | https://github.com/rust-lang/rust |
| just | mise | Test recipe | https://github.com/casey/just |
| ketch | see its README | Installs swarfr | https://github.com/pyrlyn/ketch |
| swarfr | ketch | Lossless cleanup of target/ after tests | https://github.com/listepo/swarfr |
| bwrap | system package | Linux backend at run time (not needed to build) | https://github.com/containers/bubblewrap |
| sandbox-exec | macOS | Seatbelt backend at run time | https://developer.apple.com/documentation/security/app_sandbox |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| thiserror | local | https://github.com/dtolnay/thiserror | `ConfineError` |
| landlock | local (Linux only) | https://github.com/landlock-lsm/rust-landlock | Filesystem rules for the Landlock backend |
| seccompiler | local (Linux only) | https://github.com/rust-vmm/seccompiler | Network filter for the Landlock backend |
| nix | local (Linux only) | https://github.com/nix-rust/nix | `libc` re-export for the syscall numbers and errno |
| tempfile | local | https://github.com/Stebalien/tempfile | Test fixtures |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
