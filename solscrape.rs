//! Solscrape - A CLI tool to scrape Solidity source code from git repositories
//! 
//! Compile: rustc -O solscrape.rs -o solscrape
//! Or with Cargo (see docs.md)

use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

// ============================================================================
// Configuration
// ============================================================================

const VERSION: &str = "1.0.0";

const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "lib",
    "out",
    "cache",
    "artifacts",
    "build",
    "coverage",
    "test",
    "tests",
    "script",
    "scripts",
    "dependencies",
    ".deps",
];

// ============================================================================
// CLI Argument Parsing
// ============================================================================

#[derive(Debug)]
struct Args {
    source: String,
    destination: String,
    output_name: Option<String>,
    is_local: bool,
    include_lib: bool,
    include_test: bool,
    include_script: bool,
    no_headers: bool,
    quiet: bool,
    show_help: bool,
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

fn print_version() {
    println!("solscrape {}", VERSION);
}

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

#[derive(Debug, Clone, Copy, PartialEq)]
enum ParserState {
    Normal,
    InStringDouble,
    InStringSingle,
    InSingleComment,
    InMultiComment,
}

/// Removes all comments from Solidity source code while preserving string literals.
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

/// Removes empty lines and trims trailing whitespace from each line.
fn remove_empty_lines(code: &str) -> String {
    code.lines()
        .map(|line| line.trim_end())
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<&str>>()
        .join("\n")
}

/// Full cleaning pipeline: remove comments and empty lines.
fn clean_solidity(code: &str) -> String {
    let without_comments = remove_comments(code);
    remove_empty_lines(&without_comments)
}

// ============================================================================
// Git Operations
// ============================================================================

/// Clones a git repository to the target directory using shallow clone.
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
fn extract_repo_name(url: &str) -> String {
    let url = url.trim_end_matches('/');
    let url = url.trim_end_matches(".git");
    url.rsplit('/').next().unwrap_or("repository").to_string()
}

// ============================================================================
// File Discovery
// ============================================================================

/// Recursively finds all Solidity files in a directory.
fn find_solidity_files(dir: &Path, excluded: &HashSet<&str>) -> io::Result<Vec<PathBuf>> {
    let mut sol_files = Vec::new();
    find_solidity_files_recursive(dir, excluded, &mut sol_files)?;
    sol_files.sort();
    Ok(sol_files)
}

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

/// Builds the set of excluded directories based on configuration.
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

/// Processes a single Solidity file and returns cleaned content with optional header.
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

struct ScraperResult {
    output_path: PathBuf,
    file_count: usize,
    line_count: usize,
    files_processed: Vec<String>,
}

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

mod tempfile {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        pub fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_single_line_comments() {
        let code = "uint256 x; // this is a comment\nuint256 y;";
        let result = remove_comments(code);
        assert!(!result.contains("this is a comment"));
        assert!(result.contains("uint256 x;"));
        assert!(result.contains("uint256 y;"));
    }

    #[test]
    fn test_remove_multi_line_comments() {
        let code = "uint256 x; /* multi\nline\ncomment */ uint256 y;";
        let result = remove_comments(code);
        assert!(!result.contains("multi"));
        assert!(!result.contains("comment"));
        assert!(result.contains("uint256 x;"));
        assert!(result.contains("uint256 y;"));
    }

    #[test]
    fn test_preserve_strings() {
        let code = r#"string s = "// not a comment";"#;
        let result = remove_comments(code);
        assert!(result.contains("// not a comment"));
    }

    #[test]
    fn test_preserve_strings_multiline() {
        let code = r#"string s = "/* not a comment */";"#;
        let result = remove_comments(code);
        assert!(result.contains("/* not a comment */"));
    }

    #[test]
    fn test_remove_empty_lines() {
        let code = "line1\n\n\nline2\n  \nline3";
        let result = remove_empty_lines(code);
        assert_eq!(result, "line1\nline2\nline3");
    }

    #[test]
    fn test_extract_repo_name() {
        assert_eq!(
            extract_repo_name("https://github.com/user/repo.git"),
            "repo"
        );
        assert_eq!(
            extract_repo_name("https://github.com/user/repo"),
            "repo"
        );
        assert_eq!(
            extract_repo_name("https://github.com/user/my-project.git/"),
            "my-project"
        );
    }

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
