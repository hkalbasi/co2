use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    ops::Range,
    sync::{Mutex, OnceLock},
};

use chumsky::span::Span as SpanTrait;

use crate::diagnostic::get_source_text;

// Type definitions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(u32);

impl FileId {
    pub const INVALID: Self = Self(65000);
}

impl From<usize> for FileId {
    fn from(value: usize) -> Self {
        FileId(value as u32)
    }
}

pub type SpanData = chumsky::span::SimpleSpan<usize, FileId>;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    file_id_or_tag: u16,
    start_or_intern_key: u32,
    length: u16,
}

impl SpanTrait for Span {
    type Context = <SpanData as SpanTrait>::Context;

    type Offset = <SpanData as SpanTrait>::Offset;

    fn new(context: Self::Context, range: Range<Self::Offset>) -> Self {
        Self::new(SpanData::new(context, range))
    }

    fn context(&self) -> Self::Context {
        self.data().context()
    }

    fn start(&self) -> Self::Offset {
        self.data().start()
    }

    fn end(&self) -> Self::Offset {
        self.data().end()
    }
}

impl Debug for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.data())
    }
}

impl Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.data())
    }
}

const INTERNED_TAG: u16 = u16::MAX;

struct InternTable {
    data: Vec<SpanData>,
    dedup: HashMap<SpanData, u32>,
}

fn intern_table() -> &'static Mutex<InternTable> {
    static TABLE: OnceLock<Mutex<InternTable>> = OnceLock::new();
    TABLE.get_or_init(|| {
        Mutex::new(InternTable {
            data: Vec::new(),
            dedup: HashMap::new(),
        })
    })
}

impl Span {
    pub fn from_parts(file_id: FileId, range: Range<usize>) -> Self {
        Self::new(SpanData::new(file_id, range))
    }

    pub fn new(mut data: SpanData) -> Self {
        // TODO: this if is very wrong if happen. We should panic here and fix rest of the code.
        if data.start > data.end {
            std::mem::swap(&mut data.start, &mut data.end);
        }
        if let Ok(file_id_or_tag) = u16::try_from(data.context.0)
            && file_id_or_tag != INTERNED_TAG
            && let Ok(length) = u16::try_from(data.end - data.start)
            && let Ok(start) = u32::try_from(data.start)
        {
            Self {
                file_id_or_tag,
                start_or_intern_key: start,
                length,
            }
        } else {
            let mut table = intern_table().lock().unwrap();
            let key = if let Some(&key) = table.dedup.get(&data) {
                key
            } else {
                let key = table.data.len() as u32;
                table.data.push(data);
                table.dedup.insert(data, key);
                key
            };
            Self {
                file_id_or_tag: INTERNED_TAG,
                start_or_intern_key: key,
                length: 0,
            }
        }
    }

    pub fn data(self) -> SpanData {
        if self.file_id_or_tag == INTERNED_TAG {
            let table = intern_table().lock().unwrap();
            table.data[self.start_or_intern_key as usize]
        } else {
            SpanData::new(
                FileId(self.file_id_or_tag.into()),
                self.start_or_intern_key as usize
                    ..self.start_or_intern_key as usize + self.length as usize,
            )
        }
    }

    pub fn source_text(self) -> Option<String> {
        get_source_text(self)
    }
}

pub type Spanned<T> = (T, Span);
