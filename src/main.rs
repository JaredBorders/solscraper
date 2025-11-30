//! A CLI tool for scraping and consolidating Solidity source code from git repositories.
//!
//! This crate provides [`run`] as the main entry point for extracting Solidity files
//! from either remote git repositories or local directories, cleaning them by removing
//! comments and empty lines, and combining them into a single consolidated output file.
//!
//! # Examples
//!
//! ```bash
//! # Scrape from a remote repository
//! solscrape https://github.com/OpenZeppelin/openzeppelin-contracts.git
//!
//! # Scrape from a local directory
//! solscrape ./my-project --local -o my_contracts
//!
//! # Include test and library files
//! solscrape https://github.com/example/repo.git --include-lib --include-test
//! ```
//!
//! # Features
//!
//! - **Git integration**: Clones repositories with shallow depth for fast scraping
//! - **Comment stripping**: Removes single-line (`//`) and multi-line (`/* */`) comments
//! - **String preservation**: Correctly handles comment-like syntax within string literals
//! - **Configurable exclusions**: Optionally include/exclude lib, test, and script directories
//! - **Zero dependencies**: Uses only the Rust standard library
//!
//! # Design Notes
//!
//! This binary uses `#![forbid(unsafe_code)]` and has no external dependencies.
//! A custom [`tempfile`] module provides temporary directory management with
//! automatic cleanup on drop.

#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

// ============================================================================
// Configuration
// ============================================================================

/// The current version of solscrape, following semantic versioning.
const VERSION: &str = "1.0.0";

// ============================================================================
// CLI Argument Parsing
// ============================================================================

/// Parsed command-line arguments for configuring scraper behavior.
///
/// Use [`parse_args`] to construct this from `std::env::args()`. The struct
/// provides sensible defaults via its [`Default`] implementation.
///
/// # Behavioral Contract
///
/// - Either `show_help` or `show_version` being `true` short-circuits validation;
///   `source` may be empty in these cases.
/// - When `is_local` is `false`, `source` is treated as a git URL.
/// - The `destination` defaults to the current directory (`"."`).
///
/// # Examples
///
/// ```rust,ignore
/// let args = parse_args()?;
/// if args.show_help {
///     print_help();
///     return Ok(());
/// }
/// ```
#[derive(Debug)]
struct Args {
    /// Git repository URL or local directory path to scrape.
    source: String,
    /// Output directory for the consolidated Solidity file.
    destination: String,
    /// Custom base name for the output file (without `_scraped.sol` suffix).
    output_name: Option<String>,
    /// When `true`, treat `source` as a local filesystem path instead of a git URL.
    is_local: bool,
    /// Include `lib/` directory contents in output.
    include_lib: bool,
    /// Include `test/` and `tests/` directory contents in output.
    include_test: bool,
    /// Include `script/` and `scripts/` directory contents in output.
    include_script: bool,
    /// Omit file separator headers from the consolidated output.
    no_headers: bool,
    /// Suppress progress output; only print the final output path.
    quiet: bool,
    /// Display help message and exit.
    show_help: bool,
    /// Display version information and exit.
    show_version: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            source: String::new(),
            destination: ".".to_string(),
            output_name: None,
            is_local: false,
            include_lib: false,
            include_test: false,
            include_script: false,
            no_headers: false,
            quiet: false,
            show_help: false,
            show_version: false,
        }
    }
}

/// Parses command-line arguments into a structured [`Args`] configuration.
///
/// Use this function at program startup to extract and validate CLI options.
/// For invalid arguments, returns an error message suitable for display to users.
///
/// # Returns
///
/// A populated [`Args`] struct on success, or a descriptive error message.
///
/// # Errors
///
/// | Error | Condition |
/// |-------|-----------|
/// | `"--output requires a value"` | `-o`/`--output` flag provided without argument |
/// | `"Unknown option: {arg}"` | Unrecognized flag starting with `-` |
/// | `"Missing required argument: <source>"` | No source path/URL provided |
/// | `"Too many positional arguments"` | More than two positional arguments |
///
/// # Examples
///
/// ```rust,ignore
/// let args = parse_args()?;
/// println!("Scraping from: {}", args.source);
/// ```
fn parse_args() -> Result<Args, String> {
    let args: Vec<String> = env::args().collect();
    let mut parsed = Args::default();
    let mut positional: Vec<String> = Vec::new();
    let mut i = 1;

    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" => parsed.show_help = true,
            "-v" | "--version" => parsed.show_version = true,
            "-l" | "--local" => parsed.is_local = true,
            "-q" | "--quiet" => parsed.quiet = true,
            "--include-lib" => parsed.include_lib = true,
            "--include-test" => parsed.include_test = true,
            "--include-script" => parsed.include_script = true,
            "--no-headers" => parsed.no_headers = true,
            "-o" | "--output" => {
                i += 1;
                if i >= args.len() {
                    return Err("--output requires a value".to_string());
                }
                parsed.output_name = Some(args[i].clone());
            }
            _ if arg.starts_with('-') => {
                return Err(format!("Unknown option: {}", arg));
            }
            _ => positional.push(arg.clone()),
        }
        i += 1;
    }

    if parsed.show_help || parsed.show_version {
        return Ok(parsed);
    }

    match positional.len() {
        0 => return Err("Missing required argument: <source>".to_string()),
        1 => parsed.source = positional[0].clone(),
        2 => {
            parsed.source = positional[0].clone();
            parsed.destination = positional[1].clone();
        }
        _ => return Err("Too many positional arguments".to_string()),
    }

    Ok(parsed)
}

/// Prints the help message with usage instructions and available options.
///
/// Displays comprehensive CLI documentation including argument descriptions,
/// all available flags, and practical usage examples.
fn print_help() {
    println!(
        r#"
Solscrape v{} - Solidity Source Code Scraper

USAGE:
    solscrape [OPTIONS] <source> [destination]

ARGUMENTS:
    <source>        Git repository URL or local directory path (with --local)
    [destination]   Output directory (default: current directory)

OPTIONS:
    -o, --output <NAME>    Custom output filename (without _scraped.sol suffix)
    -l, --local            Treat source as a local directory path
    --include-lib          Include lib/ dependencies
    --include-test         Include test/ files
    --include-script       Include script/ files
    --no-headers           Omit file separator headers in output
    -q, --quiet            Suppress progress output (only print result path)
    -h, --help             Show this help message
    -v, --version          Show version

EXAMPLES:
    solscrape https://github.com/clober-dex/v2-core.git
    solscrape https://github.com/OpenZeppelin/openzeppelin-contracts.git ./output
    solscrape https://github.com/uniswap/v3-core.git -o uniswap_v3
    solscrape ./my-local-project --local -o my_contracts
    solscrape https://github.com/example/repo.git --include-lib --include-test
"#,
        VERSION
    );
}

/// Prints the version string in `solscrape {VERSION}` format.
fn print_version() {
    println!("solscrape {}", VERSION);
}

/// Prints the decorative ASCII banner with version information.
///
/// Displayed at startup in non-quiet mode to provide visual context.
fn print_banner() {
    println!(
        r#"
╔═══════════════════════════════════════════════════════════════╗
║              SOLSCRAPE v{}  -  Solidity Scraper              ║
╚═══════════════════════════════════════════════════════════════╝
"#,
        VERSION
    );
}

// ============================================================================
// Solidity Parser - Comment Removal
// ============================================================================

/// The current parsing context within Solidity source code.
///
/// Used by [`remove_comments`] to track whether the parser is inside a string
/// literal, comment block, or normal code context. This enables correct handling
/// of comment-like syntax within strings (e.g., `"// not a comment"`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum ParserState {
    /// Normal code context; comments and strings may begin.
    Normal,
    /// Inside a double-quoted string literal (`"..."`).
    InStringDouble,
    /// Inside a single-quoted string literal (`'...'`).
    InStringSingle,
    /// Inside a single-line comment (`// ...`).
    InSingleComment,
    /// Inside a multi-line comment (`/* ... */`).
    InMultiComment,
}

/// Removes all comments from Solidity source code while preserving string literals.
///
/// Use this function to strip both single-line (`//`) and multi-line (`/* */`)
/// comments from Solidity code. String literals containing comment-like syntax
/// are correctly preserved.
///
/// # Behavioral Contract
///
/// - Single-line comments are removed up to (but not including) the newline
/// - Multi-line comments are removed entirely, including their delimiters
/// - Escape sequences within strings (e.g., `\"`) are handled correctly
/// - The output length is always less than or equal to the input length
///
/// # Examples
///
/// ```rust,ignore
/// let code = "uint256 x; // comment\nuint256 y;";
/// let cleaned = remove_comments(code);
/// assert_eq!(cleaned, "uint256 x; \nuint256 y;");
/// ```
///
/// String literals are preserved:
///
/// ```rust,ignore
/// let code = r#"string s = "// not a comment";"#;
/// let cleaned = remove_comments(code);
/// assert!(cleaned.contains("// not a comment"));
/// ```
fn remove_comments(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    let mut result = String::with_capacity(code.len());
    let mut state = ParserState::Normal;
    let mut i = 0;

    while i < chars.len() {
        match state {
            ParserState::Normal => {
                // Check for string start
                if chars[i] == '"' {
                    state = ParserState::InStringDouble;
                    result.push(chars[i]);
                    i += 1;
                } else if chars[i] == '\'' {
                    state = ParserState::InStringSingle;
                    result.push(chars[i]);
                    i += 1;
                }
                // Check for single-line comment
                else if i + 1 < chars.len() && chars[i] == '/' && chars[i + 1] == '/' {
                    state = ParserState::InSingleComment;
                    i += 2;
                }
                // Check for multi-line comment
                else if i + 1 < chars.len() && chars[i] == '/' && chars[i + 1] == '*' {
                    state = ParserState::InMultiComment;
                    i += 2;
                }
                // Normal character
                else {
                    result.push(chars[i]);
                    i += 1;
                }
            }

            ParserState::InStringDouble => {
                result.push(chars[i]);
                // Handle escape sequences
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    result.push(chars[i]);
                    i += 1;
                } else if chars[i] == '"' {
                    state = ParserState::Normal;
                    i += 1;
                } else {
                    i += 1;
                }
            }

            ParserState::InStringSingle => {
                result.push(chars[i]);
                // Handle escape sequences
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    result.push(chars[i]);
                    i += 1;
                } else if chars[i] == '\'' {
                    state = ParserState::Normal;
                    i += 1;
                } else {
                    i += 1;
                }
            }

            ParserState::InSingleComment => {
                if chars[i] == '\n' {
                    // Keep the newline, exit comment
                    result.push('\n');
                    state = ParserState::Normal;
                }
                i += 1;
            }

            ParserState::InMultiComment => {
                if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '/' {
                    state = ParserState::Normal;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        }
    }

    result
}

/// Removes empty lines and trailing whitespace from source code.
///
/// Use this function to normalize whitespace after comment removal. Each line
/// is trimmed of trailing whitespace, and lines that are empty or contain only
/// whitespace are removed entirely.
///
/// # Examples
///
/// ```rust,ignore
/// let code = "line1\n\n\nline2\n  \nline3";
/// let cleaned = remove_empty_lines(code);
/// assert_eq!(cleaned, "line1\nline2\nline3");
/// ```
fn remove_empty_lines(code: &str) -> String {
    code.lines()
        .map(|line| line.trim_end())
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<&str>>()
        .join("\n")
}

/// Cleans Solidity source code by removing comments and empty lines.
///
/// This is the primary cleaning pipeline that combines [`remove_comments`] and
/// [`remove_empty_lines`] into a single operation. Use this for preparing
/// Solidity files for consolidation.
///
/// # Examples
///
/// ```rust,ignore
/// let code = r#"
/// // SPDX-License-Identifier: MIT
/// pragma solidity ^0.8.0;
///
/// /* Multi-line comment */
/// contract Test {
///     uint256 value;
/// }
/// "#;
/// let cleaned = clean_solidity(code);
/// assert!(!cleaned.contains("SPDX"));
/// assert!(cleaned.contains("pragma solidity"));
/// ```
fn clean_solidity(code: &str) -> String {
    let without_comments = remove_comments(code);
    remove_empty_lines(&without_comments)
}

// ============================================================================
// Git Operations
// ============================================================================

/// Clones a git repository to the specified directory using shallow clone.
///
/// Uses `git clone --depth 1` for minimal bandwidth and disk usage. The target
/// directory is created if it doesn't exist.
///
/// # Arguments
///
/// * `url` — The git repository URL (HTTPS or SSH format)
/// * `target_dir` — The filesystem path where the repository will be cloned
///
/// # Errors
///
/// | Error | Condition |
/// |-------|-----------|
/// | `"Git is not installed..."` | `git` command not found in PATH |
/// | `"Failed to execute git: {e}"` | System error spawning the git process |
/// | `"Git clone failed: {stderr}"` | Git returned non-zero exit code |
///
/// # Examples
///
/// ```rust,ignore
/// let temp = tempfile::tempdir()?;
/// clone_repository("https://github.com/user/repo.git", temp.path())?;
/// ```
fn clone_repository(url: &str, target_dir: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .args(["clone", "--depth", "1", url])
        .arg(target_dir)
        .output()
        .map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                "Git is not installed or not in PATH. Please install Git first.".to_string()
            } else {
                format!("Failed to execute git: {}", e)
            }
        })?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Git clone failed: {}", stderr.trim()))
    }
}

/// Extracts the repository name from a git URL.
///
/// Parses the final path component from a git URL, stripping trailing slashes
/// and the `.git` extension. Use this to derive a default output filename when
/// none is specified.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(extract_repo_name("https://github.com/user/repo.git"), "repo");
/// assert_eq!(extract_repo_name("https://github.com/user/repo"), "repo");
/// assert_eq!(extract_repo_name("https://github.com/user/my-project.git/"), "my-project");
/// ```
fn extract_repo_name(url: &str) -> String {
    let url = url.trim_end_matches('/');
    let url = url.trim_end_matches(".git");
    url.rsplit('/').next().unwrap_or("repository").to_string()
}

// ============================================================================
// File Discovery
// ============================================================================

/// Recursively discovers all Solidity files in a directory tree.
///
/// Walks the directory tree starting from `dir`, collecting paths to all `.sol`
/// files while respecting the exclusion set. Results are sorted alphabetically
/// for deterministic output ordering.
///
/// # Arguments
///
/// * `dir` — The root directory to search
/// * `excluded` — Directory names to skip (e.g., `"node_modules"`, `"lib"`)
///
/// # Returns
///
/// A sorted vector of absolute paths to Solidity files.
///
/// # Errors
///
/// Returns an I/O error if the directory cannot be read.
///
/// # Examples
///
/// ```rust,ignore
/// let excluded: HashSet<&str> = [".git", "node_modules"].into_iter().collect();
/// let files = find_solidity_files(Path::new("./contracts"), &excluded)?;
/// ```
fn find_solidity_files(dir: &Path, excluded: &HashSet<&str>) -> io::Result<Vec<PathBuf>> {
    let mut sol_files = Vec::new();
    find_solidity_files_recursive(dir, excluded, &mut sol_files)?;
    sol_files.sort();
    Ok(sol_files)
}

/// Recursive helper for [`find_solidity_files`].
///
/// Traverses subdirectories depth-first, appending found `.sol` file paths to
/// the accumulator. Directories matching names in `excluded` are skipped.
fn find_solidity_files_recursive(
    dir: &Path,
    excluded: &HashSet<&str>,
    files: &mut Vec<PathBuf>,
) -> io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();

        if path.is_dir() {
            if !excluded.contains(name_str.as_ref()) {
                find_solidity_files_recursive(&path, excluded, files)?;
            }
        } else if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "sol" {
                    files.push(path);
                }
            }
        }
    }

    Ok(())
}

/// Builds the set of directory names to exclude from scraping.
///
/// Creates a [`HashSet`] of directory names that should be skipped during
/// file discovery. Some directories are always excluded (e.g., `.git`,
/// `node_modules`), while others depend on the [`Args`] configuration.
///
/// # Arguments
///
/// * `args` — The parsed CLI arguments containing inclusion flags
///
/// # Always Excluded
///
/// - `.git`, `node_modules`, `out`, `cache`, `artifacts`
/// - `build`, `coverage`, `.deps`, `dependencies`
///
/// # Conditionally Excluded
///
/// | Directory | Included When |
/// |-----------|---------------|
/// | `lib/` | `args.include_lib` is `true` |
/// | `test/`, `tests/` | `args.include_test` is `true` |
/// | `script/`, `scripts/` | `args.include_script` is `true` |
fn build_excluded_dirs(args: &Args) -> HashSet<&'static str> {
    let mut excluded: HashSet<&str> = HashSet::new();

    // Always exclude these
    excluded.insert(".git");
    excluded.insert("node_modules");
    excluded.insert("out");
    excluded.insert("cache");
    excluded.insert("artifacts");
    excluded.insert("build");
    excluded.insert("coverage");
    excluded.insert(".deps");
    excluded.insert("dependencies");

    // Conditionally exclude based on flags
    if !args.include_lib {
        excluded.insert("lib");
    }
    if !args.include_test {
        excluded.insert("test");
        excluded.insert("tests");
        excluded.insert("Test");
        excluded.insert("Tests");
    }
    if !args.include_script {
        excluded.insert("script");
        excluded.insert("scripts");
        excluded.insert("Script");
        excluded.insert("Scripts");
    }

    excluded
}

// ============================================================================
// File Processing
// ============================================================================

/// Processes a single Solidity file and returns its cleaned content.
///
/// Reads the file, applies [`clean_solidity`] to remove comments and empty lines,
/// and optionally prepends a decorative header showing the file's relative path.
///
/// # Arguments
///
/// * `path` — Absolute path to the Solidity file
/// * `base_dir` — Base directory for computing relative paths in headers
/// * `add_header` — Whether to include a file separator header in the output
///
/// # Returns
///
/// - `Ok(Some(content))` — The cleaned file content (with optional header)
/// - `Ok(None)` — The file was empty after cleaning
/// - `Err(e)` — The file could not be read
///
/// # Examples
///
/// ```rust,ignore
/// let content = process_file(
///     Path::new("/project/src/Token.sol"),
///     Path::new("/project"),
///     true
/// )?;
/// ```
fn process_file(path: &Path, base_dir: &Path, add_header: bool) -> io::Result<Option<String>> {
    let content = fs::read_to_string(path)?;
    let cleaned = clean_solidity(&content);

    if cleaned.trim().is_empty() {
        return Ok(None);
    }

    let relative_path = path
        .strip_prefix(base_dir)
        .unwrap_or(path)
        .to_string_lossy();

    if add_header {
        let separator = "// ══════════════════════════════════════════════════════════════════════";
        Ok(Some(format!(
            "{}\n// File: {}\n{}\n{}",
            separator, relative_path, separator, cleaned
        )))
    } else {
        Ok(Some(cleaned))
    }
}

// ============================================================================
// Main Scraper
// ============================================================================

/// The result of a successful scraping operation.
///
/// Contains statistics about the scraping process and the location of the
/// output file. Use this to report results to the user or for programmatic
/// access to the output.
struct ScraperResult {
    /// The absolute path to the generated consolidated Solidity file.
    output_path: PathBuf,
    /// The number of Solidity files that were processed.
    file_count: usize,
    /// The total number of lines in the consolidated output.
    line_count: usize,
    /// Relative paths of all files that were included in the output.
    files_processed: Vec<String>,
}

/// Scrapes Solidity files from a directory and consolidates them into a single file.
///
/// This is the core scraping logic used by both [`scrape_from_url`] and
/// [`scrape_from_local`]. It discovers Solidity files, processes each one,
/// and writes the combined output to the destination directory.
///
/// # Arguments
///
/// * `source_dir` — The directory containing Solidity files to scrape
/// * `destination` — The output directory for the consolidated file
/// * `output_name` — Base name for the output file (produces `{name}_scraped.sol`)
/// * `args` — Configuration affecting which files to include
///
/// # Returns
///
/// A [`ScraperResult`] containing statistics and the output path.
///
/// # Errors
///
/// | Error | Condition |
/// |-------|-----------|
/// | `"Failed to scan directory: {e}"` | I/O error during file discovery |
/// | `"No Solidity files found..."` | No `.sol` files in the source tree |
/// | `"All Solidity files were empty..."` | All files were empty after cleaning |
/// | `"Failed to create destination: {e}"` | Cannot create output directory |
/// | `"Failed to create output file: {e}"` | Cannot create the output file |
/// | `"Failed to write output: {e}"` | Error writing to the output file |
fn scrape_directory(
    source_dir: &Path,
    destination: &str,
    output_name: &str,
    args: &Args,
) -> Result<ScraperResult, String> {
    let excluded = build_excluded_dirs(args);

    // Find all Solidity files
    let sol_files = find_solidity_files(source_dir, &excluded)
        .map_err(|e| format!("Failed to scan directory: {}", e))?;

    if sol_files.is_empty() {
        return Err("No Solidity files found in the source".to_string());
    }

    // Process all files
    let mut all_parts: Vec<String> = Vec::new();
    let mut files_processed: Vec<String> = Vec::new();

    for file_path in &sol_files {
        let relative = file_path
            .strip_prefix(source_dir)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        match process_file(file_path, source_dir, !args.no_headers) {
            Ok(Some(content)) => {
                all_parts.push(content);
                files_processed.push(relative);
            }
            Ok(None) => {
                // Empty file, skip
            }
            Err(e) => {
                if !args.quiet {
                    eprintln!("Warning: Could not read {}: {}", relative, e);
                }
            }
        }
    }

    if all_parts.is_empty() {
        return Err("All Solidity files were empty after processing".to_string());
    }

    // Combine all code
    let final_code = all_parts.join("\n");
    let line_count = final_code.lines().count();

    // Prepare output path
    let dest_path = Path::new(destination);
    fs::create_dir_all(dest_path).map_err(|e| format!("Failed to create destination: {}", e))?;

    let output_filename = format!("{}_scraped.sol", output_name);
    let output_path = dest_path.join(&output_filename);

    // Write output
    let mut file =
        File::create(&output_path).map_err(|e| format!("Failed to create output file: {}", e))?;

    file.write_all(final_code.as_bytes())
        .map_err(|e| format!("Failed to write output: {}", e))?;

    Ok(ScraperResult {
        output_path,
        file_count: files_processed.len(),
        line_count,
        files_processed,
    })
}

/// Scrapes Solidity files from a remote git repository.
///
/// Clones the repository to a temporary directory, processes all Solidity files,
/// and writes the consolidated output. The temporary directory is automatically
/// cleaned up when the function returns.
///
/// # Arguments
///
/// * `url` — The git repository URL to clone
/// * `destination` — The output directory for the consolidated file
/// * `output_name` — Optional custom output name; defaults to repository name
/// * `args` — Configuration affecting scraping behavior
///
/// # Returns
///
/// A [`ScraperResult`] on success.
///
/// # Errors
///
/// Returns an error if cloning fails or if the scraping process encounters errors.
/// See [`clone_repository`] and [`scrape_directory`] for specific error conditions.
///
/// # Examples
///
/// ```rust,ignore
/// let result = scrape_from_url(
///     "https://github.com/OpenZeppelin/openzeppelin-contracts.git",
///     "./output",
///     Some("openzeppelin"),
///     &args
/// )?;
/// println!("Output: {}", result.output_path.display());
/// ```
fn scrape_from_url(
    url: &str,
    destination: &str,
    output_name: Option<&str>,
    args: &Args,
) -> Result<ScraperResult, String> {
    // Create temporary directory
    let temp_dir = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {}", e))?;

    let temp_path = temp_dir.path();

    if !args.quiet {
        println!("Cloning repository...");
    }

    clone_repository(url, temp_path)?;

    if !args.quiet {
        println!("Processing files...");
    }

    let name = output_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| extract_repo_name(url));

    scrape_directory(temp_path, destination, &name, args)
}

/// Scrapes Solidity files from a local directory.
///
/// Processes all Solidity files in the specified local path and writes the
/// consolidated output. Unlike [`scrape_from_url`], this operates directly on
/// the filesystem without cloning.
///
/// # Arguments
///
/// * `path` — The local directory path containing Solidity files
/// * `destination` — The output directory for the consolidated file
/// * `output_name` — Optional custom output name; defaults to directory name
/// * `args` — Configuration affecting scraping behavior
///
/// # Returns
///
/// A [`ScraperResult`] on success.
///
/// # Errors
///
/// | Error | Condition |
/// |-------|-----------|
/// | `"Source path does not exist: {path}"` | The specified path doesn't exist |
/// | `"Source path is not a directory: {path}"` | The path is a file, not a directory |
///
/// Additional errors may come from [`scrape_directory`].
///
/// # Examples
///
/// ```rust,ignore
/// let result = scrape_from_local(
///     "./my-project",
///     "./output",
///     Some("my_contracts"),
///     &args
/// )?;
/// ```
fn scrape_from_local(
    path: &str,
    destination: &str,
    output_name: Option<&str>,
    args: &Args,
) -> Result<ScraperResult, String> {
    let source_path = Path::new(path);

    if !source_path.exists() {
        return Err(format!("Source path does not exist: {}", path));
    }

    if !source_path.is_dir() {
        return Err(format!("Source path is not a directory: {}", path));
    }

    if !args.quiet {
        println!("Scanning local directory...");
    }

    let name = output_name.map(|s| s.to_string()).unwrap_or_else(|| {
        source_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "local".to_string())
    });

    scrape_directory(source_path, destination, &name, args)
}

// ============================================================================
// Temporary Directory (simple implementation)
// ============================================================================

/// A minimal temporary directory implementation with automatic cleanup.
///
/// This module provides `TempDir` and `tempdir()` as a zero-dependency
/// alternative to the `tempfile` crate. Temporary directories are automatically
/// removed when the `TempDir` is dropped.
///
/// # Design Notes
///
/// Directory names are generated using nanosecond timestamps to ensure uniqueness.
/// The cleanup on drop uses best-effort semantics—errors are silently ignored.
mod tempfile {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A temporary directory that is automatically removed on drop.
    ///
    /// Created via [`tempdir`], this struct owns a directory in the system's
    /// temporary directory. The directory and all its contents are deleted
    /// when this struct is dropped.
    ///
    /// # Lifecycle
    ///
    /// - **Construction** ([`tempdir`]): Creates a new directory with a unique name
    /// - **Clone**: Not implemented; temporary directories are single-owner
    /// - **Drop**: Recursively deletes the directory and all contents
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let temp = tempdir()?;
    /// let file_path = temp.path().join("data.txt");
    /// std::fs::write(&file_path, "hello")?;
    /// // Directory is deleted when `temp` goes out of scope
    /// ```
    pub struct TempDir {
        /// The absolute path to the temporary directory.
        path: PathBuf,
    }

    impl TempDir {
        /// The path to this temporary directory.
        pub fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    /// Creates a new temporary directory with a unique name.
    ///
    /// The directory is created in the system's temporary directory (e.g., `/tmp`
    /// on Unix) with a name in the format `solscrape_{timestamp}`.
    ///
    /// # Returns
    ///
    /// A [`TempDir`] that will be automatically cleaned up on drop.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the directory cannot be created.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let temp = tempdir()?;
    /// println!("Using temp dir: {}", temp.path().display());
    /// ```
    pub fn tempdir() -> std::io::Result<TempDir> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let temp_base = std::env::temp_dir();
        let dir_name = format!("solscrape_{}", timestamp);
        let path = temp_base.join(dir_name);

        fs::create_dir_all(&path)?;

        Ok(TempDir { path })
    }
}

// ============================================================================
// Entry Point
// ============================================================================

/// Executes the main scraping workflow based on command-line arguments.
///
/// This is the core application logic, separated from `main` to enable proper
/// error handling with the `?` operator. It parses arguments, performs the
/// scraping operation, and prints results.
///
/// # Returns
///
/// - `Ok(())` — Scraping completed successfully (or help/version was shown)
/// - `Err(message)` — An error occurred; message is suitable for display
///
/// # Behavioral Contract
///
/// - If `--help` or `--version` is passed, prints the requested info and returns `Ok`
/// - In quiet mode, only the output path is printed to stdout
/// - In normal mode, a banner, progress messages, and summary are printed
fn run() -> Result<(), String> {
    let args = parse_args()?;

    if args.show_help {
        print_help();
        return Ok(());
    }

    if args.show_version {
        print_version();
        return Ok(());
    }

    if !args.quiet {
        print_banner();
        println!("Source:      {}", args.source);
        println!("Destination: {}", args.destination);
        println!();
    }

    let output_name = args.output_name.as_deref();

    let result = if args.is_local {
        scrape_from_local(&args.source, &args.destination, output_name, &args)?
    } else {
        scrape_from_url(&args.source, &args.destination, output_name, &args)?
    };

    if args.quiet {
        println!("{}", result.output_path.display());
    } else {
        println!();
        println!("════════════════════════════════════════════════════════════════");
        println!("✅ Success!");
        println!("   Files processed: {}", result.file_count);
        println!("   Total lines:     {}", result.line_count);
        println!("   Output:          {}", result.output_path.display());
        println!("════════════════════════════════════════════════════════════════");

        if result.file_count <= 25 {
            println!("\nFiles included:");
            for f in &result.files_processed {
                println!("  • {}", f);
            }
        }
    }

    Ok(())
}

/// The program entry point.
///
/// Delegates to [`run`] for the main logic and converts the result into an
/// appropriate [`ExitCode`]. Errors are printed to stderr with a visual indicator.
///
/// # Exit Codes
///
/// - `0` — Success
/// - `1` — Any error occurred
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("❌ Error: {}", e);
            ExitCode::FAILURE
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Unit tests for the Solidity scraper.
///
/// These tests verify the core parsing and utility functions. Integration tests
/// for the full scraping workflow require filesystem access and are not included.
#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that single-line comments are removed from code.
    #[test]
    fn test_remove_single_line_comments() {
        let code = "uint256 x; // this is a comment\nuint256 y;";
        let result = remove_comments(code);
        assert!(!result.contains("this is a comment"));
        assert!(result.contains("uint256 x;"));
        assert!(result.contains("uint256 y;"));
    }

    /// Verifies that multi-line comments are removed from code.
    #[test]
    fn test_remove_multi_line_comments() {
        let code = "uint256 x; /* multi\nline\ncomment */ uint256 y;";
        let result = remove_comments(code);
        assert!(!result.contains("multi"));
        assert!(!result.contains("comment"));
        assert!(result.contains("uint256 x;"));
        assert!(result.contains("uint256 y;"));
    }

    /// Verifies that comment-like syntax within double-quoted strings is preserved.
    #[test]
    fn test_preserve_strings() {
        let code = r#"string s = "// not a comment";"#;
        let result = remove_comments(code);
        assert!(result.contains("// not a comment"));
    }

    /// Verifies that multi-line comment syntax within strings is preserved.
    #[test]
    fn test_preserve_strings_multiline() {
        let code = r#"string s = "/* not a comment */";"#;
        let result = remove_comments(code);
        assert!(result.contains("/* not a comment */"));
    }

    /// Verifies that empty and whitespace-only lines are removed.
    #[test]
    fn test_remove_empty_lines() {
        let code = "line1\n\n\nline2\n  \nline3";
        let result = remove_empty_lines(code);
        assert_eq!(result, "line1\nline2\nline3");
    }

    /// Verifies repository name extraction from various URL formats.
    #[test]
    fn test_extract_repo_name() {
        assert_eq!(
            extract_repo_name("https://github.com/user/repo.git"),
            "repo"
        );
        assert_eq!(extract_repo_name("https://github.com/user/repo"), "repo");
        assert_eq!(
            extract_repo_name("https://github.com/user/my-project.git/"),
            "my-project"
        );
    }

    /// Verifies the complete cleaning pipeline with realistic Solidity code.
    #[test]
    fn test_clean_solidity() {
        let code = r#"
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/* This is a 
   multi-line comment */
contract Test {
    // Single line comment
    uint256 public value;
    
    string public name = "// not removed";
}
"#;
        let result = clean_solidity(code);

        assert!(!result.contains("SPDX-License-Identifier"));
        assert!(!result.contains("multi-line comment"));
        assert!(!result.contains("Single line comment"));
        assert!(result.contains("pragma solidity"));
        assert!(result.contains("contract Test"));
        assert!(result.contains("uint256 public value"));
        assert!(result.contains(r#""// not removed""#));
    }
}
