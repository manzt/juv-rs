use crate::notebook::{Notebook, NotebookBuilder};
use crate::printer::Printer;
use crate::script::Runtime;
use anyhow::{bail, Result};
use clap::ValueEnum;
use once_cell::sync::Lazy;
use owo_colors::OwoColorize;
use regex::Regex;
use std::fmt::Write as _;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::NamedTempFile;

#[derive(ValueEnum, Debug, Clone, PartialEq)]
#[clap(rename_all = "kebab_case")]
pub(crate) enum RunMode {
    Managed,
    Replace,
    Dry,
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    printer: &Printer,
    path: &Path,
    with: &[String],
    python: Option<&str>,
    jupyter: Option<&str>,
    jupyter_args: &[String],
    mode: RunMode,
    no_project: bool,
) -> Result<()> {
    let runtime: Runtime = jupyter.unwrap_or("lab").parse()?;
    let notebook = Notebook::from_path(path)?;

    let meta = notebook.as_ref().cells.iter().find_map(|cell| {
        if let nbformat::v4::Cell::Code { source, .. } = cell {
            PEP723_REGEX
                .captures(&source.join(""))
                .and_then(|cap| cap.get(0).map(|m| m.as_str().to_string()))
        } else {
            None
        }
    });

    // TODO: Support managed version
    let is_managed = false;
    let script = runtime.run_script(path, meta.as_deref(), is_managed, jupyter_args);

    let mut command = Command::new("uv");
    command.stdout(Stdio::inherit());
    command.stderr(Stdio::inherit());
    command
        .arg("run")
        .arg("--with")
        .arg(runtime.as_dependency_specifier())
        .arg("-");
    if no_project {
        command.arg("--no-project");
    }
    if let Some(python) = python {
        command.arg("--python").arg(python);
    }
    for with_item in with {
        command.arg("--with").arg(with_item);
    }

    if mode == RunMode::Dry {
        let args: Vec<_> = command
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        println!("uv {}", args.join(" "));
        return Ok(());
    }

    command.stdin(Stdio::piped());
    let mut child = command.spawn()?;
    let stdin = child.stdin.as_mut().expect("Failed to open stdin");
    stdin.write_all(script.as_bytes())?;

    let status = child.wait()?;
    if !status.success() {
        writeln!(
            printer.stderr(),
            "{}: uv command failed with exit code {}",
            "error".red().bold(),
            status.code().unwrap_or(-1)
        )?;
        std::process::exit(1);
    }

    Ok(())
}

pub fn exec(
    _printer: &Printer,
    path: &Path,
    python: Option<&str>,
    with: &[String],
    quiet: bool,
) -> Result<()> {
    let path = std::path::absolute(path)?;
    let mut args = vec!["run", "-"];
    if quiet {
        args.push("--quiet");
    }
    if let Some(python) = python {
        args.push("--python");
        args.push(python);
    }
    for with_item in with {
        args.push("--with");
        args.push(with_item);
    }

    let mut child = Command::new("uv")
        .args(&args)
        .current_dir(path.parent().unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    {
        let mut stdin = child
            .stdin
            .as_ref()
            .map(BufWriter::new)
            .expect("Failed to open stdin");
        let nb = Notebook::from_path(path.as_ref())?;
        write_script(&mut stdin, nb.as_ref())?;
    }

    let status = child.wait()?;
    if !status.success() {
        println!(
            "{}: uv command failed with exit code {}",
            "error".red().bold(),
            status.code().unwrap_or(-1)
        );
        std::process::exit(1);
    }

    Ok(())
}

pub fn init(printer: &Printer, path: Option<&Path>, python: Option<&str>) -> Result<()> {
    let path = match path {
        Some(p) => p.to_path_buf(),
        None => get_first_non_conflicting_untitled_ipybnb(&std::env::current_dir()?)?,
    };
    let path = std::path::absolute(&path)?;
    let dir = path.parent().expect("path must have a parent");

    if path.extension().and_then(|s| s.to_str()) != Some("ipynb") {
        writeln!(
            printer.stderr(),
            "{}: The notebook must have a `{}` extension",
            "error".red().bold(),
            ".ipynb".cyan()
        )?;
        std::process::exit(1);
    }

    let nb = new_notebook_with_inline_metadata(dir, python)?;
    std::fs::write(&path, serde_json::to_string_pretty(nb.as_ref())?)?;

    writeln!(
        printer.stdout(),
        "Initialized notebook at `{}`",
        path.strip_prefix(dir)?.display().cyan()
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn add(
    printer: &Printer,
    path: &Path,
    packages: &[String],
    requirements: Option<&Path>,
    extras: &[String],
    tag: Option<&str>,
    branch: Option<&str>,
    rev: Option<&str>,
    editable: bool,
) -> Result<()> {
    let mut nb = Notebook::from_path(path)?;

    for cell in nb.as_mut().cells.iter_mut() {
        match cell {
            nbformat::v4::Cell::Code { source, .. } if PEP723_REGEX.is_match(&source.join("")) => {
                let temp_file = tempfile::Builder::new()
                    .suffix(".py")
                    .tempfile_in(path.parent().unwrap())?;

                std::fs::write(temp_file.path(), source.join("").trim())?;

                let mut command = Command::new("uv");
                command.arg("add").arg("--script").arg(temp_file.path());

                if editable {
                    command.arg("--editable");
                }

                if let Some(requirements) = requirements {
                    command.arg("--requirements").arg(requirements);
                }

                if let Some(tag) = tag {
                    command.arg("--tag").arg(tag);
                }

                if let Some(branch) = branch {
                    command.arg("--branch").arg(branch);
                }

                if let Some(rev) = rev {
                    command.arg("--rev").arg(rev);
                }

                for extra in extras {
                    command.arg("--extra").arg(extra);
                }

                command.args(packages);

                let output = command.output()?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    anyhow::bail!("uv command failed: {}", stderr);
                }

                let contents = std::fs::read_to_string(temp_file.path())?;
                *source = contents
                    .trim()
                    .split_inclusive('\n')
                    .map(|s| s.to_string())
                    .collect();

                break;
            }
            _ => {}
        }
    }

    std::fs::write(path, serde_json::to_string_pretty(nb.as_ref())?)?;
    writeln!(printer.stderr(), "Updated `{}`", path.display().cyan())?;
    Ok(())
}

pub fn edit(printer: &Printer, file: &Path, editor: Option<&str>) -> Result<()> {
    let nb = Notebook::from_path(file)?;
    let mut temp_file = tempfile::Builder::new().suffix(".md").tempfile()?;
    {
        let mut buffer = BufWriter::new(&mut temp_file);
        write_markdown(&mut buffer, nb.as_ref())?;
        buffer.flush()?;
    }

    let status = match editor {
        Some(editor) => Command::new(editor).arg(temp_file.path()).status()?,
        None => {
            writeln!(
                printer.stderr(),
                "{}: No editor specified. Please set the EDITOR environment variable or use the `{}` flag.",
                "error".red().bold(),
                "--editor".yellow().bold()
            )?;
            std::process::exit(1);
        }
    };

    if !status.success() {
        writeln!(
            printer.stderr(),
            "{}: Editor command failed with exit code {}",
            "error".red().bold(),
            status.code().unwrap_or(-1)
        )?;
        std::process::exit(1);
    }

    let update = std::fs::read_to_string(temp_file.path())?;

    println!("{}", update);

    // TODO: Need to parse the markdown "cell" contents and update the corresponding cells

    Ok(())
}

pub fn clear(printer: &Printer, targets: &[String], check: bool) -> Result<()> {
    let mut paths: Vec<PathBuf> = Vec::new();

    // Collect notebook paths from the specified targets
    for target in targets {
        let path = Path::new(target);
        if path.is_dir() {
            // Use glob to find .ipynb files in directory
            glob::glob(&format!("{}/*.ipynb", path.display()))?.for_each(|entry| {
                if let Ok(notebook_path) = entry {
                    paths.push(notebook_path);
                }
            });
        } else if path.is_file() && path.extension().map_or(false, |ext| ext == "ipynb") {
            paths.push(path.to_path_buf());
        } else {
            writeln!(
                printer.stderr(),
                "{}: Skipping `{}` because it is not a notebook",
                "warning".yellow().bold(),
                path.display().cyan(),
            )?;
        }
    }

    if check {
        let mut any_not_cleared = false;

        // Check each notebook to see if it is already cleared
        for path in &paths {
            let notebook = Notebook::from_path(path)?;
            if !notebook.is_cleared() {
                writeln!(printer.stderr(), "{}", path.display().magenta())?;
                any_not_cleared = true;
            }
        }

        if any_not_cleared {
            writeln!(
                printer.stderr(),
                "{}: Some notebooks are not cleared. Use {} to fix.",
                "error".red(),
                "juv clear".yellow().bold(),
            )?;
            std::process::exit(1);
        } else {
            writeln!(printer.stderr(), "All notebooks are cleared")?;
        }
    } else {
        // Clear the outputs in each notebook
        for path in &paths {
            let mut notebook = Notebook::from_path(path)?;
            notebook.clear_cells()?;
            std::fs::write(path, serde_json::to_string_pretty(notebook.as_ref())?)?;
            writeln!(
                printer.stderr(),
                "Cleared output from `{}`",
                path.display().cyan()
            )?;
        }
        if paths.len() > 1 {
            writeln!(
                printer.stderr(),
                "Cleared output from {} notebooks",
                paths.len().to_string().cyan().bold()
            )?;
        }
    }

    Ok(())
}

pub fn cat(
    _printer: &Printer,
    file: &std::path::Path,
    script: bool,
    pager: Option<&str>,
) -> Result<()> {
    let nb = Notebook::from_path(file)?;
    let mut writer: Box<dyn Write> = match pager.map(str::trim) {
        Some("") | None => Box::new(BufWriter::new(io::stdout().lock())),
        Some(pager) => {
            let mut command = Command::new(pager);
            if pager == "bat" {
                let ext = if script { "py" } else { "md" };
                // special case `bat` to add additional flags
                command
                    .arg("--language")
                    .arg(ext)
                    .arg("--file-name")
                    .arg(format!(
                        "{}.{}",
                        file.file_stem()
                            .unwrap_or("stdin".as_ref())
                            .to_string_lossy(),
                        ext
                    ));
            }
            let child = command.stdin(Stdio::piped()).spawn()?;
            // Ok to unwrap because we know we set stdin to piped
            Box::new(BufWriter::new(child.stdin.unwrap()))
        }
    };

    if script {
        write_script(&mut writer, nb.as_ref())?;
    } else {
        write_markdown(&mut writer, nb.as_ref())?;
    };

    writer.flush()?;

    Ok(())
}

fn write_script(writer: &mut impl Write, nb: &nbformat::v4::Notebook) -> Result<()> {
    for (i, cell) in nb.cells.iter().enumerate() {
        if i > 0 {
            // Add a newline between cells
            writer.write_all(b"\n\n")?;
        }
        match cell {
            nbformat::v4::Cell::Code { source, .. } => {
                writer.write_all(b"# %%\n")?;
                for line in source.iter() {
                    writer.write_all(line.as_bytes())?;
                }
            }
            nbformat::v4::Cell::Markdown { source, .. } => {
                writer.write_all(b"# %% [markdown]\n")?;
                for line in source.iter() {
                    writer.write_all(b"# ")?;
                    writer.write_all(line.as_bytes())?;
                }
            }
            nbformat::v4::Cell::Raw { source, .. } => {
                writer.write_all(b"# %% [raw]\n")?;
                for line in source.iter() {
                    writer.write_all(b"# ")?;
                    writer.write_all(line.as_bytes())?;
                }
            }
        }
    }
    Ok(())
}

fn write_markdown(writer: &mut impl Write, nb: &nbformat::v4::Notebook) -> Result<()> {
    for (i, cell) in nb.cells.iter().enumerate() {
        if i > 0 {
            // Add a newline between cells
            writer.write_all(b"\n\n")?;
        }
        match cell {
            nbformat::v4::Cell::Code { source, .. } => {
                writer.write_all(b"```python\n")?;
                for line in source.iter() {
                    writer.write_all(line.as_bytes())?;
                }
                writer.write_all(b"\n```")?;
            }
            nbformat::v4::Cell::Markdown { source, .. } => {
                for line in source.iter() {
                    writer.write_all(line.as_bytes())?;
                }
            }
            nbformat::v4::Cell::Raw { source, .. } => {
                writer.write_all(b"```\n")?;
                for line in source.iter() {
                    writer.write_all(line.as_bytes())?;
                }
                writer.write_all(b"\n```")?;
            }
        }
    }
    Ok(())
}

fn get_first_non_conflicting_untitled_ipybnb(directory: &Path) -> Result<PathBuf> {
    let base_name = "Untitled";
    let extension = "ipynb";

    if !directory
        .join(format!("{}.{}", base_name, extension))
        .exists()
    {
        return Ok(directory.join(format!("{}.{}", base_name, extension)));
    }

    for i in 1..100 {
        let file_name = format!("{}{}.{}", base_name, i, extension);
        let path = directory.join(&file_name);
        if !path.exists() {
            return Ok(path);
        }
    }

    bail!("Could not find an available UntitledX.ipynb");
}

fn new_notebook_with_inline_metadata(directory: &Path, python: Option<&str>) -> Result<Notebook> {
    let temp_file = NamedTempFile::new_in(directory)?;
    let temp_path = temp_file.path().to_path_buf();

    let mut command = Command::new("uv");

    command
        .arg("init")
        .arg("--script")
        .arg(temp_path.to_str().unwrap());

    if let Some(py) = python {
        command.arg("--python").arg(py);
    }

    let output = command.output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("uv command failed: {}", stderr);
    }

    Ok(NotebookBuilder::new()
        .hidden_code_cell(&std::fs::read_to_string(temp_path)?)
        .code_cell("")
        .build())
}

static PEP723_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^# /// (?P<type>[a-zA-Z0-9-]+)$\s(?P<content>(^#(| .*)$\s)+)^# ///$").unwrap()
});
