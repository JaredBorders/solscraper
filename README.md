# Solscrape Documentation

A fast CLI tool written in Rust to scrape and consolidate Solidity smart contract
source code from git repositories into a single analysis-ready file.

---

## Table of Contents

1. [Overview](#overview)
2. [Installation](#installation)
3. [Usage](#usage)
4. [Options Reference](#options-reference)
5. [Examples](#examples)
6. [Output Format](#output-format)
7. [How It Works](#how-it-works)
8. [Troubleshooting](#troubleshooting)

---

## Overview

**Solscrape** extracts all Solidity source code from a repository and combines it
into a single `.sol` file with:

- ✅ All comments removed (single-line `//` and multi-line `/* */`)
- ✅ Empty lines stripped
- ✅ String literals preserved (comments inside strings are kept)
- ✅ Clean, analysis-ready output

### Use Cases

- **Security audits**: Quickly review all contract logic in one file
- **Code analysis**: Feed consolidated code to analysis tools
- **Documentation**: Generate a single-file view of a protocol
- **AI/LLM analysis**: Provide complete codebase context without clutter

---

## Installation

### Prerequisites

**Git must be installed on your system.**

Check if Git is installed:

```bash
git --version
```

If not installed:

- **macOS**: `brew install git` or download from [git-scm.com](https://git-scm.com)
- **Linux**: `sudo apt install git` (Debian/Ubuntu) or `sudo dnf install git` (Fedora)
- **Windows**: Download from [git-scm.com](https://git-scm.com/download/win)

## Usage

### Basic Syntax

```
solscrape [OPTIONS] <source> [destination]
```

### Quick Start

```bash
# Scrape a GitHub repository
solscrape https://github.com/clober-dex/v2-core.git

# Specify output directory
solscrape https://github.com/OpenZeppelin/openzeppelin-contracts.git ./output

# Scrape a local project
solscrape ./my-foundry-project --local
```

---

## Options Reference

| Option             | Short | Description                                     |
| ------------------ | ----- | ----------------------------------------------- |
| `--help`           | `-h`  | Show help message                               |
| `--version`        | `-v`  | Show version                                    |
| `--output <NAME>`  | `-o`  | Custom output filename (without `_scraped.sol`) |
| `--local`          | `-l`  | Treat source as local directory path            |
| `--include-lib`    |       | Include `lib/` dependencies                     |
| `--include-test`   |       | Include `test/` files                           |
| `--include-script` |       | Include `script/` files                         |
| `--no-headers`     |       | Omit file separator headers                     |
| `--quiet`          | `-q`  | Minimal output (only print result path)         |

### Default Excluded Directories

By default, these directories are **excluded**:

```
.git, node_modules, lib, out, cache, artifacts, build,
coverage, test, tests, script, scripts, dependencies, .deps
```

Use `--include-lib`, `--include-test`, `--include-script` to include those.

---

## Examples

### Basic Repository Scraping

```bash
# Clone and scrape, output to current directory
solscrape https://github.com/clober-dex/v2-core.git
# Creates: ./v2-core_scraped.sol
```

### Custom Output Location

```bash
# Output to specific directory
solscrape https://github.com/uniswap/v3-core.git ./audits
# Creates: ./audits/v3-core_scraped.sol
```

### Custom Output Filename

```bash
# Custom name
solscrape https://github.com/uniswap/v3-core.git -o uniswap_analysis
# Creates: ./uniswap_analysis_scraped.sol
```

### Include Dependencies

```bash
# Include OpenZeppelin and other lib/ dependencies
solscrape https://github.com/example/defi-protocol.git --include-lib
```

### Include Everything

```bash
# Include lib, test, and script files
solscrape https://github.com/example/repo.git \
    --include-lib \
    --include-test \
    --include-script
```

### Local Project

```bash
# Scrape a local Foundry/Hardhat project
solscrape ./my-project --local

# With custom output
solscrape /path/to/contracts --local -o my_contracts
```

### Scripting / Automation

```bash
# Quiet mode - only outputs the file path
OUTPUT=$(solscrape https://github.com/example/repo.git -q)
echo "Scraped to: $OUTPUT"

# Use in a pipeline
solscrape https://github.com/example/repo.git -q | xargs wc -l
```

### No Headers (Pure Code)

```bash
# Remove file separator comments for pure code output
solscrape https://github.com/example/repo.git --no-headers
```

---

## Output Format

### With Headers (Default)

```solidity
// ══════════════════════════════════════════════════════════════════════
// File: src/core/Pool.sol
// ══════════════════════════════════════════════════════════════════════
pragma solidity ^0.8.19;
import "./interfaces/IPool.sol";
contract Pool is IPool {
    mapping(address => uint256) public balances;
    function deposit() external payable {
        balances[msg.sender] += msg.value;
    }
    function withdraw(uint256 amount) external {
        require(balances[msg.sender] >= amount);
        balances[msg.sender] -= amount;
        payable(msg.sender).transfer(amount);
    }
}
// ══════════════════════════════════════════════════════════════════════
// File: src/interfaces/IPool.sol
// ══════════════════════════════════════════════════════════════════════
pragma solidity ^0.8.19;
interface IPool {
    function deposit() external payable;
    function withdraw(uint256 amount) external;
}
```

### Without Headers (`--no-headers`)

```solidity
pragma solidity ^0.8.19;
import "./interfaces/IPool.sol";
contract Pool is IPool {
    mapping(address => uint256) public balances;
    function deposit() external payable {
        balances[msg.sender] += msg.value;
    }
    function withdraw(uint256 amount) external {
        require(balances[msg.sender] >= amount);
        balances[msg.sender] -= amount;
        payable(msg.sender).transfer(amount);
    }
}
pragma solidity ^0.8.19;
interface IPool {
    function deposit() external payable;
    function withdraw(uint256 amount) external;
}
```

---

## How It Works

### Processing Pipeline

```
1. INPUT
   ├── Git URL → Clone to temp directory (shallow clone)
   └── Local Path → Use directly

2. DISCOVERY
   ├── Recursively walk directory
   ├── Filter out excluded directories
   └── Collect all .sol files

3. PARSING (per file)
   ├── State machine parser
   ├── Track: Normal | InString | InSingleComment | InMultiComment
   ├── Remove comments while preserving strings
   └── Strip empty lines

4. OUTPUT
   ├── Concatenate all processed files
   ├── Add file headers (optional)
   └── Write to destination
```

### Comment Removal Algorithm

The parser uses a state machine to correctly handle:

- Single-line comments: `// comment`
- Multi-line comments: `/* comment */`
- String literals: `"// not a comment"` and `'/* preserved */'`
- Escape sequences: `"escaped \" quote"`

```
State Machine:

    ┌─────────────────────────────────────┐
    │                                     │
    ▼                                     │
 NORMAL ──"──> IN_STRING_DOUBLE ──"───────┘
    │                │
    │     '          │             \" (escape)
    │                ▼
    │         (stay in string)
    │
    ├─────'──> IN_STRING_SINGLE ─────'──> NORMAL
    │
    ├────//─> IN_SINGLE_COMMENT ───\n──> NORMAL
    │
    └────/*─> IN_MULTI_COMMENT ────*/──> NORMAL
```

---

## Troubleshooting

### "Git is not installed or not in PATH"

**Problem**: The `git` command is not available.

**Solutions**:

1. Install Git:

   - macOS: `brew install git`
   - Ubuntu: `sudo apt install git`
   - Windows: Download from git-scm.com

2. Verify installation:

   ```bash
   git --version
   ```

3. If installed but not in PATH, add it:
   - Find where git is installed
   - Add that directory to your PATH environment variable

### "Failed to clone repository"

**Possible causes**:

- Invalid URL
- Private repository (needs authentication)
- Network issues

**Solutions**:

- Verify the URL is correct and accessible
- For private repos, use SSH URL: `git@github.com:user/repo.git`
- Check your internet connection

### "No Solidity files found"

**Possible causes**:

- Repository has no `.sol` files
- All files are in excluded directories

**Solutions**:

- Verify the repository contains Solidity files
- Use `--include-lib`, `--include-test` if files are in those directories
- Check if files use `.sol` extension

### "Permission denied" on output

**Solution**:

```bash
# Ensure destination directory is writable
mkdir -p ./output
solscrape https://github.com/example/repo.git ./output
```

### Large Output File

If the output is very large:

- Don't use `--include-lib` unless necessary (OpenZeppelin alone is huge)
- Consider scraping only specific subdirectories by cloning first:
  ```bash
  git clone --depth 1 https://github.com/example/repo.git
  solscrape ./repo/src --local -o just_src
  ```

---

## Performance

Typical performance on a modern machine:

| Repository Size       | Files | Time |
| --------------------- | ----- | ---- |
| Small (< 20 files)    | ~20   | < 1s |
| Medium (50-100 files) | ~100  | 1-2s |
| Large (OpenZeppelin)  | ~300  | 3-5s |

_Note: Clone time depends on network speed and repository size._

---

## License

MIT License - Feel free to use, modify, and distribute.
