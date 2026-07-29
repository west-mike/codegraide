use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub enum InventoryError {
    Io {
        context: String,
        source: io::Error,
    },
    Configuration {
        path: Option<PathBuf>,
        message: String,
    },
    InvalidInput {
        message: String,
    },
    CategoryConflict {
        path: PathBuf,
        selector: String,
        categories: Vec<String>,
    },
}

impl InventoryError {
    pub(crate) fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    pub(crate) fn configuration(path: Option<PathBuf>, message: impl Into<String>) -> Self {
        Self::Configuration {
            path,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput {
            message: message.into(),
        }
    }
}

impl fmt::Display for InventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { context, source } => write!(formatter, "{context}: {source}"),
            Self::Configuration {
                path: Some(path),
                message,
            } => write!(
                formatter,
                "invalid configuration {}: {message}",
                path.display()
            ),
            Self::Configuration {
                path: None,
                message,
            } => write!(formatter, "invalid embedded configuration: {message}"),
            Self::InvalidInput { message } => write!(formatter, "invalid input: {message}"),
            Self::CategoryConflict {
                path,
                selector,
                categories,
            } => write!(
                formatter,
                "category conflict for {}: {selector} matches {}",
                path.display(),
                categories.join(", ")
            ),
        }
    }
}

impl Error for InventoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Configuration { .. }
            | Self::InvalidInput { .. }
            | Self::CategoryConflict { .. } => None,
        }
    }
}
