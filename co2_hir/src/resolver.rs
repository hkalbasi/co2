use rustc_public_generative::rustc_public::{
    CrateDefType,
    ty::{FnDef, Span as RustSpan, Ty},
};

#[derive(Clone, Debug)]
pub enum ResolvedValue {
    Fn(FnDef),
}

impl ResolvedValue {
    pub(crate) fn ty(&self) -> Ty {
        match self {
            ResolvedValue::Fn(fn_def) => fn_def.ty(),
        }
    }
}

type ParserSpan = co2_parser::Span;

pub struct HirCtx<'a, R> {
    resolver: &'a R,
    resolve_value: fn(&R, &str) -> Option<ResolvedValue>,
    resolve_type: fn(&R, &str) -> Option<Ty>,
    span_converter: &'a dyn Fn(ParserSpan) -> RustSpan,
}

impl<'a, R> HirCtx<'a, R> {
    pub fn new(
        resolver: &'a R,
        resolve_value: fn(&R, &str) -> Option<ResolvedValue>,
        resolve_type: fn(&R, &str) -> Option<Ty>,
        span_converter: &'a dyn Fn(ParserSpan) -> RustSpan,
    ) -> Self {
        Self {
            resolver,
            resolve_value,
            resolve_type,
            span_converter,
        }
    }

    pub(crate) fn resolve_value(&self, path: &str) -> Option<ResolvedValue> {
        (self.resolve_value)(self.resolver, path)
    }

    pub(crate) fn resolve_type(&self, path: &str) -> Option<Ty> {
        (self.resolve_type)(self.resolver, path)
    }

    pub(crate) fn to_rust_span(&self, span: ParserSpan) -> RustSpan {
        (self.span_converter)(span)
    }
}
