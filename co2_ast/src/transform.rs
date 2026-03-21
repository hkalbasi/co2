use crate::{
    DeclarationSpecifier, Declarator, Designator, EnumSpecifier, Enumerator, Expression,
    Initializer, InitializerItem, ParameterList, Spanned, SpecifierQualifier, StructDeclarator,
    StructOrUnionField, StructOrUnionKind, StructOrUnionSpecifier, TypeName, TypeResolver,
    TypeSpecifier,
};

pub trait Transformable<F: TypeResolver>: TypeResolver {
    fn transform_decl_ident(a: &F::DeclarationIdent) -> Self::DeclarationIdent;
    fn transform_struct_or_union_specifier(
        &self,
        kind: StructOrUnionKind,
        specifier: &Spanned<F::StructOrUnionIdentifier>,
    ) -> Spanned<Self::StructOrUnionIdentifier>;
    fn transform_enum_specifier(
        &self,
        specifier: &Spanned<F::EnumIdentifier>,
    ) -> Spanned<Self::EnumIdentifier>;
    fn transform_enumerator(
        &self,
        specifier: &Spanned<F::EnumeratorIdentifier>,
    ) -> Spanned<Self::EnumeratorIdentifier>;
    fn transform_path(
        &self,
        path: &Spanned<F::ResolvedRustPath>,
    ) -> Spanned<Self::ResolvedRustPath>;
}

pub trait DoTransform {
    type Resolver: TypeResolver;
    type Target<T: TypeResolver>;

    fn transform<B: Transformable<Self::Resolver>>(&self, b: &B) -> Self::Target<B>;
}

impl<I> DoTransform for Spanned<I>
where
    I: DoTransform,
{
    type Resolver = I::Resolver;
    type Target<T: TypeResolver> = Spanned<I::Target<T>>;

    fn transform<B: Transformable<Self::Resolver>>(&self, b: &B) -> Self::Target<B> {
        (self.0.transform(b), self.1)
    }
}

impl<I, J> DoTransform for (I, J)
where
    I: DoTransform,
    J: DoTransform<Resolver = I::Resolver>,
{
    type Resolver = I::Resolver;
    type Target<T: TypeResolver> = (I::Target<T>, J::Target<T>);

    fn transform<B: Transformable<Self::Resolver>>(&self, b: &B) -> Self::Target<B> {
        (self.0.transform(b), self.1.transform(b))
    }
}

impl<I> DoTransform for Vec<I>
where
    I: DoTransform,
{
    type Resolver = I::Resolver;
    type Target<T: TypeResolver> = Vec<I::Target<T>>;

    fn transform<B: Transformable<Self::Resolver>>(&self, b: &B) -> Self::Target<B> {
        self.iter().map(|x| x.transform(b)).collect()
    }
}

impl<I> DoTransform for Box<I>
where
    I: DoTransform,
{
    type Resolver = I::Resolver;
    type Target<T: TypeResolver> = Box<I::Target<T>>;

    fn transform<B: Transformable<Self::Resolver>>(&self, b: &B) -> Self::Target<B> {
        Box::new((&**self).transform(b))
    }
}

impl<I> DoTransform for Option<I>
where
    I: DoTransform,
{
    type Resolver = I::Resolver;
    type Target<T: TypeResolver> = Option<I::Target<T>>;

    fn transform<B: Transformable<Self::Resolver>>(&self, b: &B) -> Self::Target<B> {
        self.as_ref().map(|x| x.transform(b))
    }
}

impl<A: TypeResolver> DoTransform for Initializer<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = Initializer<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> Initializer<B> {
        match self {
            Initializer::Expr(e) => Initializer::Expr(e.transform(b)),
            Initializer::List(items) => {
                Initializer::List(items.iter().map(|x| x.transform(b)).collect())
            }
        }
    }
}

impl<A: TypeResolver> DoTransform for InitializerItem<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = InitializerItem<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> InitializerItem<B> {
        let InitializerItem {
            designators,
            initializer,
        } = self;
        InitializerItem {
            designators: designators.transform(b),
            initializer: initializer.transform(b),
        }
    }
}

impl<A: TypeResolver> DoTransform for Designator<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = Designator<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> Designator<B> {
        match self {
            Designator::Subscript(x) => Designator::Subscript(x.transform(b)),
            Designator::Field(x) => Designator::Field(x.clone()),
        }
    }
}

impl<A: TypeResolver> DoTransform for DeclarationSpecifier<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = DeclarationSpecifier<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> DeclarationSpecifier<B> {
        match self {
            DeclarationSpecifier::TypeSpecifier(e) => {
                DeclarationSpecifier::TypeSpecifier(e.transform(b))
            }
            DeclarationSpecifier::TypeQualifier(e) => DeclarationSpecifier::TypeQualifier(*e),
            DeclarationSpecifier::StorageSpecifier(e) => DeclarationSpecifier::StorageSpecifier(*e),
        }
    }
}

impl<A: TypeResolver> DoTransform for TypeSpecifier<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = TypeSpecifier<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> TypeSpecifier<B> {
        match self {
            TypeSpecifier::Int => TypeSpecifier::Int,
            TypeSpecifier::Bool => TypeSpecifier::Bool,
            TypeSpecifier::Void => TypeSpecifier::Void,
            TypeSpecifier::Char => TypeSpecifier::Char,
            TypeSpecifier::Short => TypeSpecifier::Short,
            TypeSpecifier::Long => TypeSpecifier::Long,
            TypeSpecifier::Float => TypeSpecifier::Float,
            TypeSpecifier::Double => TypeSpecifier::Double,
            TypeSpecifier::Signed => TypeSpecifier::Signed,
            TypeSpecifier::Unsigned => TypeSpecifier::Unsigned,
            TypeSpecifier::StructOrUnion { kind, specifier } => {
                let kind = *kind;
                let specifier = b.transform_struct_or_union_specifier(kind, specifier);
                TypeSpecifier::StructOrUnion { kind, specifier }
            }
            TypeSpecifier::Enum(enum_specifier) => {
                TypeSpecifier::Enum(b.transform_enum_specifier(enum_specifier))
            }
            TypeSpecifier::TypedefName(path) => TypeSpecifier::TypedefName(b.transform_path(path)),
        }
    }
}

impl<A: TypeResolver> DoTransform for Declarator<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = Declarator<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> Declarator<B> {
        match self {
            Declarator::Abstract => Declarator::Abstract,
            Declarator::Identifier((i, span)) => {
                Declarator::Identifier((B::transform_decl_ident(i), *span))
            }
            Declarator::FunctionDeclarator {
                declarator,
                param_list,
            } => Declarator::FunctionDeclarator {
                declarator: declarator.transform(b),
                param_list: param_list.transform(b),
            },
            Declarator::PointerDeclarator {
                declarator,
                qualifiers,
            } => Declarator::PointerDeclarator {
                declarator: declarator.transform(b),
                qualifiers: qualifiers.clone(),
            },
            Declarator::ArrayDeclarator {
                declarator,
                subscription,
            } => Declarator::ArrayDeclarator {
                declarator: declarator.transform(b),
                subscription: subscription.clone(),
            },
        }
    }
}

impl<A: TypeResolver> DoTransform for ParameterList<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = ParameterList<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> ParameterList<B> {
        let ParameterList {
            parameters,
            ellipsis,
        } = self;
        ParameterList {
            parameters: parameters.transform(b),
            ellipsis: *ellipsis,
        }
    }
}

impl<A: TypeResolver> DoTransform for StructOrUnionSpecifier<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = StructOrUnionSpecifier<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> StructOrUnionSpecifier<B> {
        match self {
            StructOrUnionSpecifier::Defined { ident, fields } => StructOrUnionSpecifier::Defined {
                ident: ident.clone(),
                fields: fields.transform(b),
            },
            StructOrUnionSpecifier::Declared { ident } => StructOrUnionSpecifier::Declared {
                ident: ident.clone(),
            },
            StructOrUnionSpecifier::Anonymous { fields } => StructOrUnionSpecifier::Anonymous {
                fields: fields.transform(b),
            },
        }
    }
}

impl<A: TypeResolver> DoTransform for StructDeclarator<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = StructDeclarator<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> StructDeclarator<B> {
        let StructDeclarator { declarator, bits } = self;
        StructDeclarator {
            declarator: declarator.transform(b),
            bits: bits.clone(),
        }
    }
}

impl<A: TypeResolver> DoTransform for StructOrUnionField<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = StructOrUnionField<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> StructOrUnionField<B> {
        let StructOrUnionField {
            specifiers,
            declarators,
        } = self;
        StructOrUnionField {
            specifiers: specifiers.transform(b),
            declarators: declarators.transform(b),
        }
    }
}

impl<A: TypeResolver> DoTransform for EnumSpecifier<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = EnumSpecifier<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> EnumSpecifier<B> {
        match self {
            EnumSpecifier::Defined { ident, enumerators } => EnumSpecifier::Defined {
                ident: ident.clone(),
                enumerators: enumerators
                    .into_iter()
                    .map(|i| b.transform_enumerator(i))
                    .collect(),
            },
            EnumSpecifier::Declared { ident } => EnumSpecifier::Declared {
                ident: ident.clone(),
            },
            EnumSpecifier::Anonymous { enumerators } => EnumSpecifier::Anonymous {
                enumerators: enumerators
                    .into_iter()
                    .map(|i| b.transform_enumerator(i))
                    .collect(),
            },
        }
    }
}

impl<A: TypeResolver> DoTransform for Enumerator<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = Enumerator<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> Enumerator<B> {
        let Enumerator { ident, value } = self;
        Enumerator {
            ident: ident.clone(),
            value: value.transform(b),
        }
    }
}

impl<A: TypeResolver> DoTransform for TypeName<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = TypeName<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> TypeName<B> {
        let TypeName {
            specifier_qualifier_list,
            abstract_declarator,
        } = self;
        TypeName {
            specifier_qualifier_list: specifier_qualifier_list.transform(b),
            abstract_declarator: abstract_declarator.transform(b),
        }
    }
}

impl<A: TypeResolver> DoTransform for SpecifierQualifier<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = SpecifierQualifier<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> SpecifierQualifier<B> {
        match self {
            SpecifierQualifier::TypeSpecifier(e) => {
                SpecifierQualifier::TypeSpecifier(e.transform(b))
            }
            SpecifierQualifier::TypeQualifier(e) => SpecifierQualifier::TypeQualifier(*e),
        }
    }
}

// impl<A: TypeResolver> DoTransform for FFFFFF<A> {
//     type Resolver = A;
//     type Target<T: TypeResolver> = FFFFFF<T>;

//     fn transform<B: Transformable<A>>(&self, b: &B) -> FFFFFF<B> {

//     }
// }

impl<A: TypeResolver> DoTransform for Expression<A> {
    type Resolver = A;
    type Target<T: TypeResolver> = Expression<T>;

    fn transform<B: Transformable<A>>(&self, b: &B) -> Expression<B> {
        match self {
            Expression::Empty => Expression::Empty,
            Expression::Constant(constant) => Expression::Constant(constant.clone()),
            Expression::Identifier(path) => Expression::Identifier(b.transform_path(path)),
            Expression::Field(e, name) => Expression::Field(e.transform(b), name.clone()),
            Expression::Arrow(e, name) => Expression::Arrow(e.transform(b), name.clone()),
            Expression::Subscript(e1, e2) => {
                Expression::Subscript(e1.transform(b), e2.transform(b))
            }
            Expression::BinOp(lhs, op, rhs) => {
                Expression::BinOp(lhs.transform(b), *op, rhs.transform(b))
            }
            Expression::UnaryOp(op, e) => Expression::UnaryOp(*op, e.transform(b)),
            Expression::Conditional {
                cond,
                then_expr,
                else_expr,
            } => Expression::Conditional {
                cond: cond.transform(b),
                then_expr: then_expr.transform(b),
                else_expr: else_expr.transform(b),
            },
            Expression::Cast { type_name, expr } => Expression::Cast {
                type_name: type_name.transform(b),
                expr: expr.transform(b),
            },
            Expression::Call { func, params } => Expression::Call {
                func: func.transform(b),
                params: params.transform(b),
            },
            Expression::Update {
                expr,
                op,
                is_postfix,
            } => Expression::Update {
                expr: expr.transform(b),
                op: *op,
                is_postfix: *is_postfix,
            },
            Expression::AssignWithOp { lhs, op, rhs } => Expression::AssignWithOp {
                lhs: lhs.transform(b),
                op: *op,
                rhs: rhs.transform(b),
            },
            Expression::SizeofType(t) => Expression::SizeofType(t.transform(b)),
            Expression::Sizeof(e) => Expression::Sizeof(e.transform(b)),
            Expression::CompoundLiteral {
                type_name,
                initializer,
            } => Expression::CompoundLiteral {
                type_name: type_name.transform(b),
                initializer: initializer.transform(b),
            },
            Expression::VaArg { .. }
            | Expression::VaStart { .. }
            | Expression::VaEnd { .. }
            | Expression::GnuStatementExpr { .. } => todo!(),
        }
    }
}
