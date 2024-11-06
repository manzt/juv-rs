use crate::printer::Printer;
use anyhow::{bail, Result};
use nbformat::{
    self,
    v4::{Cell, CellMetadata, Metadata},
};
use owo_colors::OwoColorize;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::fmt::Write as _;
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
    std::fs::write(&path, serde_json::to_string_pretty(nb.as_ref())?)?;

    writeln!(
        printer.stdout(),
        "Initialized notebook at `{}`",
        path.strip_prefix(dir)?.display().cyan()
    )?;
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
        Some(editor) => {
            Command::new(editor).arg(temp_file.path()).status()?
        }
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
            std::fs::write(path, serde_json::to_string_pretty(&notebook.0)?)?;
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

    Ok(NotebookBuilder::new()
        .code_cell(&std::fs::read_to_string(temp_path)?)
        .code_cell("")
        .build())
}

struct Notebook(nbformat::v4::Notebook);

impl AsRef<nbformat::v4::Notebook> for Notebook {
    fn as_ref(&self) -> &nbformat::v4::Notebook {
        &self.0
    }
}

impl Notebook {
    fn from_path(path: &Path) -> Result<Self> {
        let json = std::fs::read_to_string(path)?;
        Ok(Self(match nbformat::parse_notebook(&json)? {
            nbformat::Notebook::V4(nb) => nb,
            nbformat::Notebook::Legacy(legacy_nb) => nbformat::upgrade_legacy_notebook(legacy_nb)?,
        }))
    }

    // Whether the notebook outputs are cleared
    fn is_cleared(&self) -> bool {
        for cell in &self.as_ref().cells {
            if let Cell::Code {
                execution_count,
                outputs,
                ..
            } = cell
            {
                if execution_count.is_some() || !outputs.is_empty() {
                    return false;
                }
            }
        }
        true
    }

    fn clear_cells(&mut self) -> Result<()> {
        for cell in &mut self.0.cells {
            if let Cell::Code {
                execution_count,
                outputs,
                ..
            } = cell
            {
                *execution_count = None;
                outputs.clear();
            }
        }
        Ok(())
    }
}

struct NotebookBuilder {
    nb: nbformat::v4::Notebook,
}

impl NotebookBuilder {
    fn new() -> Self {
        Self {
            nb: nbformat::v4::Notebook {
                nbformat: 4,
                nbformat_minor: 4,
                metadata: Metadata {
                    kernelspec: None,
                    language_info: None,
                    authors: None,
                    additional: Default::default(),
                },
                cells: vec![],
            },
        }
    }

    fn code_cell(mut self, source: &str) -> Self {
        // TODO: Could have our own builder for this as well
        let cell = Cell::Code {
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
        };
        self.nb.cells.push(cell);
        self
    }

    fn build(self) -> Notebook {
        Notebook(self.nb)
    }
}
