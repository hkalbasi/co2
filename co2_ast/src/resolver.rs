use std::fmt::Debug;

use crate::{
    Declaration, EnumSpecifier, Enumerator, LazySubscription, RustPath, Spanned, StructOrUnionKind,
    StructOrUnionSpecifier, TypeQueryResult,
};

pub trait TypeResolver: Clone + 'static {
    type ResolvedRustPath: Debug + Clone;
    type DeclarationIdent: Debug + Clone;
    type StructOrUnionIdentifier: Debug + Clone;
    type EnumIdentifier: Debug + Clone;
    type EnumeratorIdentifier: Debug + Clone;
    type SubscriptionIdentifier: Debug + Clone;

    fn classify_path(
        &self,
        path: &RustPath<StatelessResolver>,
    ) -> Option<(TypeQueryResult, Self::ResolvedRustPath)>;
    fn register_ident(&self, name: String) -> Self::DeclarationIdent;
    /// Per C11 6.2.1p7, the scope of a declared identifier begins at the end of its declarator,
    /// before its initializer. This registers a single already-parsed declarator identifier as a
    /// local expression value so it is visible when parsing subsequent initializers in the same
    /// declaration.
    fn declare_ident_as_local(&self, ident: &Self::DeclarationIdent) -> Self;
    fn register_decl(&self, decl: &Declaration<Self>) -> Self;
    fn start_new_scope(&self) -> Self;
    fn register_struct_or_union_specifier(
        &self,
        kind: StructOrUnionKind,
        specifier: Spanned<StructOrUnionSpecifier<Self>>,
    ) -> Self::StructOrUnionIdentifier;
    fn register_enumerator(
        &self,
        enumerator: Spanned<Enumerator<Self>>,
    ) -> Self::EnumeratorIdentifier;
    fn register_enum_specifier(
        &self,
        specifier: Spanned<EnumSpecifier<Self>>,
    ) -> Self::EnumIdentifier;
    fn register_subscription(
        &self,
        subscription: Spanned<LazySubscription>,
    ) -> Self::SubscriptionIdentifier;
    fn rust_style_syntax_enabled(&self) -> bool;
}

#[derive(Debug, Clone)]
pub struct StatelessResolver {
    rust_style_enabled: bool,
}

impl StatelessResolver {
    pub fn new() -> Self {
        Self::with_rust_style_enabled(true)
    }

    fn with_rust_style_enabled(rust_style_enabled: bool) -> Self {
        Self { rust_style_enabled }
    }
}

impl Default for StatelessResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeResolver for StatelessResolver {
    type ResolvedRustPath = RustPath<Self>;
    type DeclarationIdent = String;
    type StructOrUnionIdentifier = StructOrUnionSpecifier<Self>;
    type EnumIdentifier = EnumSpecifier<Self>;
    type EnumeratorIdentifier = Enumerator<Self>;
    type SubscriptionIdentifier = LazySubscription;

    fn classify_path(
        &self,
        path: &RustPath<StatelessResolver>,
    ) -> Option<(TypeQueryResult, RustPath<StatelessResolver>)> {
        Some((TypeQueryResult::Unsure, path.clone()))
    }

    fn register_ident(&self, name: String) -> Self::DeclarationIdent {
        name
    }

    fn declare_ident_as_local(&self, _ident: &String) -> Self {
        self.clone()
    }

    fn register_decl(&self, decl: &Declaration<Self>) -> Self {
        let mut rust_style_enabled = self.rust_style_enabled;
        match decl {
            Declaration::FunctionDefinition { signature, .. } => {
                rust_style_enabled &= signature.ident().as_deref() != Some("fn");
            }
            Declaration::Declaration { declarators, .. } => {
                rust_style_enabled &= declarators
                    .iter()
                    .all(|decl| decl.0.declarator.0.ident().as_deref() != Some("fn"));
            }
            Declaration::RustTypeAlias { ident, .. } => {
                rust_style_enabled &= ident.0.as_str() != "fn";
            }
            Declaration::PragmaPack { .. } => {}
        }
        StatelessResolver::with_rust_style_enabled(rust_style_enabled)
    }

    fn start_new_scope(&self) -> Self {
        self.clone()
    }

    fn register_struct_or_union_specifier(
        &self,
        _kind: StructOrUnionKind,
        spec: Spanned<StructOrUnionSpecifier<Self>>,
    ) -> Self::StructOrUnionIdentifier {
        spec.0
    }

    fn register_enumerator(
        &self,
        enumerator: Spanned<Enumerator<Self>>,
    ) -> Self::EnumeratorIdentifier {
        enumerator.0
    }

    fn register_enum_specifier(
        &self,
        specifier: Spanned<EnumSpecifier<Self>>,
    ) -> Self::EnumIdentifier {
        specifier.0
    }

    fn register_subscription(
        &self,
        subscription: Spanned<LazySubscription>,
    ) -> Self::SubscriptionIdentifier {
        subscription.0
    }

    fn rust_style_syntax_enabled(&self) -> bool {
        self.rust_style_enabled
    }
}
