use thiserror::Error;

#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum FunkyError {
    #[error("lex error at {file}:{line}:{col}: {message}")]
    Lex {
        file: String,
        line: u32,
        col: u32,
        message: String,
    },

    #[error("formatter error: {0}")]
    Format(String),

    #[error("config error in '{path}': {source}")]
    Config {
        path: String,
        #[source]
        source: toml::de::Error,
    },

    #[error("I/O error for '{path}': {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("file is not valid UTF-8: {path}")]
    NotUtf8 { path: String },
}
