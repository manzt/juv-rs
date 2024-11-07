use anyhow::Result;
use nbformat::v4::{Cell, CellId, CellMetadata, JupyterCellMetadata, Metadata};
use std::path::Path;

pub struct Notebook(nbformat::v4::Notebook);

impl AsRef<nbformat::v4::Notebook> for Notebook {
    fn as_ref(&self) -> &nbformat::v4::Notebook {
        &self.0
    }
}

impl AsMut<nbformat::v4::Notebook> for Notebook {
    fn as_mut(&mut self) -> &mut nbformat::v4::Notebook {
        &mut self.0
    }
}

impl Notebook {
    pub fn from_path(path: &Path) -> Result<Self> {
        let json = std::fs::read_to_string(path)?;
        Ok(Self(match nbformat::parse_notebook(&json)? {
            nbformat::Notebook::V4(nb) => nb,
            nbformat::Notebook::Legacy(legacy_nb) => nbformat::upgrade_legacy_notebook(legacy_nb)?,
        }))
    }

    // Whether the notebook outputs are cleared
    pub fn is_cleared(&self) -> bool {
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

    pub fn clear_cells(&mut self) -> Result<()> {
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

pub struct NotebookBuilder {
    nb: nbformat::v4::Notebook,
}

impl NotebookBuilder {
    pub fn new() -> Self {
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

    fn _code_cell(mut self, source: &str, hidden: Option<bool>) -> Self {
        let uuid = uuid::Uuid::new_v4().to_string();
        // TODO: Could have our own builder for this as well
        let cell = Cell::Code {
            // ok to unwrap because we know the first part of the uuid is valid
            id: CellId::try_from(uuid.split('-').next().unwrap()).unwrap(),
            metadata: CellMetadata {
                id: None,
                collapsed: None,
                scrolled: None,
                deletable: None,
                editable: None,
                format: None,
                jupyter: hidden.map(|h| JupyterCellMetadata {
                    source_hidden: Some(h),
                    outputs_hidden: None,
                }),
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

    pub fn hidden_code_cell(self, source: &str) -> Self {
        self._code_cell(source, Some(true))
    }

    pub fn code_cell(self, source: &str) -> Self {
        self._code_cell(source, None)
    }

    pub fn build(self) -> Notebook {
        Notebook(self.nb)
    }
}
