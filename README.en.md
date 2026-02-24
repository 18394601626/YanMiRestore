# YanMiRestore (Rust)

A data recovery tool built around a **read-only source** and **safety-first** model, suitable for:
- PC disks (HDD/SSD)
- External storage devices such as portable HDDs and USB drives
- Legally exported phone backups or disk images

## Implemented Recovery Paths

The current project includes 5 usable recovery paths:
- Logical deleted-file recovery from recycle/trash folders under mounted directories
- NTFS deleted MFT record scanning (including data runlist parsing)
- FAT12/16/32 and exFAT deleted directory entry scanning
- ext4 deleted inode scanning (including extent/direct block mapping)
- Raw image signature carving

## Project Layout (Standardized)

The `src` directory is organized into entry points plus domain modules, using directory modules (`mod.rs`) consistently:

```text
src/
|- main.rs                 # Program entry (help hint when args are empty, logger init)
|- app/mod.rs              # Command dispatch and flow orchestration
|- cli/mod.rs              # CLI argument definitions
|- config/mod.rs           # Runtime config loading (YanMiRestore.toml)
|- error/mod.rs            # Unified error types
|- model/
|  |- mod.rs               # Re-exports for domain models
|  |- enums.rs             # Enum models (target/depth/filesystem hints)
|  |- request.rs           # Request models
|  |- plan.rs              # Plan models
|  |- scan.rs              # Scan report models
|  |- recovery.rs          # Recovery session models
|  |- fs.rs                # Intermediate filesystem scan models
|  \- device.rs            # Device list models
|- scanner/mod.rs          # Scan flow
|- recovery/mod.rs         # Recovery flow
|- report/mod.rs           # Console output and report writing
|- utils/
|  |- mod.rs               # Utility module entry
|  \- label.rs             # Shared label/text utilities
|- device/mod.rs           # Device identification
|- carver/mod.rs           # Signature carving
\- fs/
   |- mod.rs               # Filesystem scan aggregation
   |- ntfs.rs
   |- fat.rs
   |- ext4.rs
   \- partition.rs
```

## Command Reference

### Recent Updates
- Progress bars now use **dynamic refresh** (default output to `stdout`), including:
  - Spinner frames (`| / - \`)
  - Elapsed time, current progress, percentage, and stage information
- Added `--skip-carved` to `recover`, allowing filesystem-only recovery candidates and avoiding carved files that may be fragmented/corrupted.
- Added `YanMiRestore.toml` for centralized settings such as scan thresholds, recovery buffer, and progress refresh frequency.

### 1. `design` (generate recovery plan)
- Generate an execution plan based on `case-id`, target type, scan depth, and filesystem hint
- Output stages, safety rules, and prerequisites

### 2. `scan` (read-only scan)
- Automatically detects whether input is a mounted directory or an image file
- With `--target-kind auto` in an interactive terminal, the tool shows detection results first and asks for `y/yes` confirmation before continuing
- Use `--yes` to skip confirmation (useful for scripts/batch jobs)
- Logical scan directories:
  - `$Recycle.Bin`, `RECYCLER`, `Recycler`, `RECYCLED`
  - `.Trash`, `.Trashes`
- Image scan capabilities:
  - NTFS: scans deleted MFT entries and parses resident/non-resident `$DATA`
  - FAT/exFAT: scans deleted directory entries and tries to rebuild file segments via cluster chains or contiguous clusters
  - ext4: scans deleted inodes where `i_dtime != 0` and extracts recoverable segments
- Partition detection:
  - Supports MBR/GPT
  - Supports extended partition and logical partition chains
  - Can probe NTFS/FAT/ext4 by partition offset
- Metadata recognized but not yet recovered:
  - APFS signature detection
  - F2FS signature detection
- Optional signature carving (`--include-carving`):
  - `jpg`, `png`, `pdf`, `sqlite`, `docx`, `xlsx`, `mp4`
- Generates a JSON scan report including candidates and recovery coordinates
- CLI prints scan summary and filesystem metrics
- CLI displays dynamic stage progress bars (device info, filesystem scan, signature scan, result summary)

### 3. `recover` (recovery output)
- Default is `dry-run`: generates planned actions only, no file writes
- Use `--execute` to perform actual export
  - Logical entries: copied from recycle/trash paths
  - NTFS/ext4 entries: extracted by `source_segments`
  - Carved entries: extracted from image by `offset + size`
- Default behavior prefers original filenames (auto-appends `_1`, `_2`, etc. for conflicts)
- Default behavior preserves source access/modified timestamps when available
- Use `--no-keep-original-name` to switch to legacy prefix naming
- Use `--no-preserve-timestamps` to disable timestamp sync
- Use `--skip-carved` to skip signature-carved candidates (helps avoid partially damaged files)
- Generates a recovery manifest file: `恢复清单.json`
- CLI shows item-by-item dynamic recovery progress and current item in real time
- In non-TTY logging environments, progress bars may degrade to plain text output

### 4. Configuration File (new)
- On startup, the tool first tries to load `YanMiRestore.toml` from the current directory
- You can also provide a config path with global option `--config <path>`
- TOML config sections:
  - `ui`: refresh frequency, animation interval, templates, characters, spinner frames, stage prefixes
  - `scan`: logical scan limits and recycle/trash directory names
  - `carver`: carving read range, per-signature item limit, size thresholds, ZIP/MP4 search windows
  - `fs.ntfs` / `fs.fat` / `fs.ext4`: filesystem scan thresholds
  - `partition`: partition parsing limits
  - `recovery`: recovery buffer and raw-volume alignment settings

## Structured Metrics

In scan reports, `fs_result.metrics` contains:
- `ntfs`: recoverable / metadata-only / compressed / encrypted / runlist-failed statistics
- `fat`: volume-type counts, deleted files/directories, segmented recovery, metadata-only statistics
- `ext4`: volume count, deleted files/directories, sparse segments, unsupported depth, legacy pointer statistics

## Safety Constraints

- The tool never writes to the source media
- Recovery outputs must be written to a destination path
- `recover` blocks these dangerous destinations:
  - Destination directory is inside source path
  - Destination directory is a parent of source path
  - Destination path equals source path
  - On Windows, destination is on the same volume as source (same drive-letter prefix)
- Reports and manifests are retained for auditing and traceability

### Common Safety Error

If you see an error like:

```text
Error: Unsafe destination: source (\\.\F:) and destination (F:\restore) are on the same Windows volume
```

it means you set the output directory on the source drive `F:`. To avoid overwriting recoverable data, the tool blocks this by design.

Set `--destination` to a different drive, for example:

```powershell
.\YanMiRestore.exe recover --report E:\output\CASE-F-001-scan-report.json --destination G:\restore --execute
```

## Usage Examples

```bash
# Method A: run directly with cargo

# 1) Generate a recovery plan (deep scan + carving)
cargo run -- design --case-id CASE-1001 --target-kind pc-disk --depth deep --include-carving

# 2) Scan a PC disk image (auto filesystem detection)
cargo run -- scan --source E:\evidence\disk.img --output .\output --case-id CASE-1001 --target-kind pc-disk --depth deep --include-carving

# 3) Scan a Linux/ext4 image (force ext4)
cargo run -- scan --source E:\evidence\linux.img --output .\output --case-id CASE-EXT4-001 --fs-hint ext4 --depth deep

# 4) Scan a phone-exported directory (logical path)
cargo run -- scan --source E:\evidence\phone-backup --output .\output --case-id CASE-PHONE-001 --target-kind phone --depth metadata

# 5) Recovery rehearsal (no file writes, plan only)
cargo run -- recover --report .\output\CASE-1001-scan-report.json --destination G:\restore

# 6) Execute actual recovery
cargo run -- recover --report .\output\CASE-1001-scan-report.json --destination G:\restore --execute

# 7) Execute recovery and disable "original name + timestamp" strategy (legacy-compatible naming)
cargo run -- recover --report .\output\CASE-1001-scan-report.json --destination G:\restore --execute --no-keep-original-name --no-preserve-timestamps

# 8) Recover filesystem-only candidates (skip carving results)
cargo run -- recover --report .\output\CASE-1001-scan-report.json --destination G:\restore --execute --skip-carved

# 9) Skip interactive confirmation in auto-detection mode
cargo run -- scan --source F:\ --target-kind auto --depth deep --include-carving --output E:\output --case-id CASE-F-001 --yes

# 10) Scan with a specific config file
cargo run -- --config .\YanMiRestore.toml scan --source E:\evidence\disk.img --output .\output --case-id CASE-CFG-001 --depth deep
```

```powershell
# Method B: package and run as exe (recommended for end users)

# 1) Build release
cargo build --release

# 2) Enter executable directory
cd .\target\release

# 3) Show help
.\YanMiRestore.exe --help

# 4) Scan an image (with carving)
.\YanMiRestore.exe scan --source E:\evidence\disk.img --output E:\output --case-id CASE-2001 --depth deep --include-carving

# 5) Execute recovery
.\YanMiRestore.exe recover --report E:\output\CASE-2001-scan-report.json --destination G:\restore --execute

# 6) Specify config file
.\YanMiRestore.exe --config E:\tools\YanMiRestore.toml scan --source E:\evidence\disk.img --output E:\output --case-id CASE-2001 --depth deep
```

```text
# Common output files

output\CASE-xxxx-scan-report.json     # Scan report (input for recover)
restore\恢复清单.json                 # Recovery execution manifest
```

## Windows Double-Click Note

- `YanMiRestore.exe` is a command-line application, not a GUI application.
- If launched by double-click with no arguments, it shows help and waits for Enter before exiting (no "flash-close").
- Recommended usage is from `cmd` or `PowerShell` using the commands above.

## Known Limitations

- Decoding for NTFS compressed/encrypted streams is not implemented yet (currently marked as metadata-only)
- Current FAT/exFAT and ext4 deleted-entry parsing is stable placeholder logic, mainly for volume identification and unified reporting
- FAT/exFAT may still recover only partial segments in heavily fragmented scenarios
- exFAT `NoFatChain` currently relies on contiguous cluster assumptions
- ext4 recovery is inode-centric and does not fully reconstruct deleted filenames yet
- ext4 legacy multilevel indirect blocks (single/double/triple indirect) are not fully parsed yet
- APFS/F2FS currently support signature recognition only, without deleted metadata parsing
- Signature carving is heuristic and may produce false positives or be affected by fragmentation
- Phone workflows are based on legally exported data only, and do not include bypassing encryption or locks

## Compliance

Use this tool only on devices and data that you are authorized to access.

## Open Source Collaboration

This repository includes baseline open-source governance files:

- License: `LICENSE`
- Contribution guide: `CONTRIBUTING.md`
- Code of conduct: `CODE_OF_CONDUCT.md`
- Security policy: `SECURITY.md`
- Templates and automation: `.github/`

## Auto Device Detection (new)

The `devices` subcommand can automatically detect local available devices and label them as:
- PC disk
- External drive
- Phone
- Other device

### Quick Start

```powershell
# 1) List devices first (recommended)
.\YanMiRestore.exe devices

# 2) Copy one detected path as source for scan
.\YanMiRestore.exe scan --source E:\ --target-kind auto --output .\output --case-id CASE-AUTO-001
```

### Notes

- With `--target-kind auto`, the tool determines device type from path and system drive characteristics.
- On Windows, it prioritizes system drive type classification (removable/fixed/network/optical, etc.).
- Scan summaries show both detected device type and detection basis.

## F: Drive Recovery Notes (important)

If the source is a drive root (for example `F:\`), the tool will automatically try to switch to the underlying volume path (for example `\\.\F:`) for scanning.

Please ensure:
- Run your terminal as **Administrator**, otherwise raw volume access may fail.
- Recovery destination must be on a **different drive letter** (for example, source `F:` and destination `E:` or `G:`).

Example:

```powershell
# Administrator PowerShell
.\YanMiRestore.exe scan --source F:\ --target-kind auto --depth deep --include-carving --output E:\output --case-id CASE-F-001

.\YanMiRestore.exe recover --report E:\output\CASE-F-001-scan-report.json --destination G:\restore --execute
```

If you want to skip manual confirmation after auto-detection, append `--yes`:

```powershell
.\YanMiRestore.exe scan --source F:\ --target-kind auto --depth deep --include-carving --output E:\output --case-id CASE-F-001 --yes
```

If the scan still returns 0 candidates, common reasons are:
- Deleted data on that drive was already overwritten
- SSD/USB controller already executed TRIM/garbage collection
- Deletion happened long ago and metadata has been reused
