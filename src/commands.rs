use crate::printer::Printer;
use anyhow::{bail, Result};
use nbformat::{
    self,
    v4::{Cell, CellMetadata, Metadata, Notebook},
};
use owo_colors::OwoColorize;
use std::fmt::Write as _;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::NamedTempFile;

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
    std::fs::write(&path, serde_json::to_string_pretty(&nb)?)?;

    writeln!(
        printer.stdout(),
        "Initialized notebook at `{}`",
        path.strip_prefix(dir)?.display().cyan()
    )?;
    Ok(())
}

pub fn cat(
    _printer: &Printer,
    file: &std::path::Path,
    script: bool,
    pager: Option<&str>,
) -> Result<()> {
    let json = std::fs::read_to_string(file)?;
    let nb = match nbformat::parse_notebook(&json)? {
        nbformat::Notebook::V4(nb) => nb,
        nbformat::Notebook::Legacy(legacy_nb) => nbformat::upgrade_legacy_notebook(legacy_nb)?,
    };

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
        write_script(&mut writer, &nb)?;
    } else {
        write_markdown(&mut writer, &nb)?;
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
            Cell::Code { source, .. } => {
                writer.write_all(b"# %%\n")?;
                for line in source.iter() {
                    writer.write_all(line.as_bytes())?;
                }
            }
            Cell::Markdown { source, .. } => {
                writer.write_all(b"# %% [markdown]\n")?;
                for line in source.iter() {
                    writer.write_all(b"# ")?;
                    writer.write_all(line.as_bytes())?;
                }
            }
            Cell::Raw { source, .. } => {
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
            Cell::Code { source, .. } => {
                writer.write_all(b"```python\n")?;
                for line in source.iter() {
                    writer.write_all(line.as_bytes())?;
                }
                writer.write_all(b"\n```")?;
            }
            Cell::Markdown { source, .. } => {
                for line in source.iter() {
                    writer.write_all(line.as_bytes())?;
                }
            }
            Cell::Raw { source, .. } => {
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

    let script = std::fs::read_to_string(temp_path)?;

    fn nbcell(source: &str) -> Cell {
        Cell::Code {
            id: uuid::Uuid::new_v4().into(),
            metadata: CellMetadata {
                id: None,
                collapsed: None,
                scrolled: None,
                deletable: None,
                editable: None,
                format: None,
                jupyter: None,
                name: None,
                tags: None,
                execution: None,
            },
            execution_count: None,
            source: source
                .trim()
                .split_inclusive('\n')
                .map(|s| s.to_string())
                .collect(),
            outputs: vec![],
        }
    }

    Ok(Notebook {
        nbformat: 4,
        nbformat_minor: 4,
        metadata: Metadata {
            kernelspec: None,
            language_info: None,
            authors: None,
            additional: Default::default(),
        },
        cells: vec![nbcell(&script), nbcell("")],
    })
}
