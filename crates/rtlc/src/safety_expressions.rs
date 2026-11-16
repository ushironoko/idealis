use crate::CoreError;
use crate::ir::{Binding, Branch, Domain, Expression, FoldOrder, Scalar, Step};
use crate::safety_values::Arena;
#[cfg(not(target_arch = "wasm32"))]
use crate::safety_values::ValueRemap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(crate) struct EvalContext<'a> {
    pub current: &'a [Scalar],
    pub next: &'a [Option<Scalar>],
    pub locals: &'a [Scalar],
    pub old: Option<Scalar>,
    pub prime: bool,
    pub lazy: Option<&'a LazyFrame<'a>>,
}

pub(crate) struct LazyFrame<'a> {
    slot: usize,
    value: &'a Expression,
    ctx: EvalContext<'a>,
    cache: std::cell::Cell<Option<Scalar>>,
    parent: Option<&'a LazyFrame<'a>>,
    stable: bool,
}

impl LazyFrame<'_> {
    fn force(&self, next: &[Option<Scalar>], arena: &mut Arena) -> Result<Scalar, CoreError> {
        if let Some(value) = self.cache.get() {
            return Ok(value);
        }
        let value = self.value.eval(EvalContext { next, ..self.ctx }, arena)?;
        if self.stable {
            self.cache.set(Some(value));
        }
        Ok(value)
    }
}

fn bind_arguments<'c>(
    index: usize,
    args: &'c [Expression],
    caller: EvalContext<'c>,
    chain: Option<&'c LazyFrame<'c>>,
    body: &'c Expression,
    locals: &'c [Scalar],
    arena: &mut Arena,
) -> Result<Scalar, CoreError> {
    let Some(value) = args.get(index) else {
        return body.eval(
            EvalContext {
                current: caller.current,
                next: caller.next,
                locals,
                old: None,
                prime: caller.prime,
                lazy: chain,
            },
            arena,
        );
    };
    let frame = LazyFrame {
        slot: index,
        value,
        ctx: caller,
        cache: std::cell::Cell::new(None),
        parent: chain,
        stable: true,
    };
    bind_arguments(index + 1, args, caller, Some(&frame), body, locals, arena)
}
impl<'a> EvalContext<'a> {
    pub fn new(current: &'a [Scalar], next: &'a [Option<Scalar>]) -> Self {
        Self {
            current,
            next,
            locals: &[],
            old: None,
            prime: false,
            lazy: None,
        }
    }
    fn read(self, slot: usize, next: bool) -> Result<Scalar, CoreError> {
        if next || self.prime {
            self.next
                .get(slot)
                .copied()
                .flatten()
                .ok_or_else(|| CoreError::protocol("Read of an unassigned next variable"))
        } else {
            self.current
                .get(slot)
                .copied()
                .ok_or_else(|| CoreError::protocol("Read of a missing current variable"))
        }
    }
}

pub(crate) fn register_memberships<'a>(
    expression: &'a Expression,
    visitor: &mut impl FnMut(&'a Expression),
) {
    fn register_type<'a>(expression: &'a Expression, visitor: &mut impl FnMut(&'a Expression)) {
        visitor(expression);
        for child in expression.children() {
            register_type(child, visitor);
        }
    }
    if let Expression::FunctionIn { right, .. } | Expression::SequenceIn { right, .. } = expression
    {
        register_type(right, visitor);
    }
    for child in expression.children() {
        register_memberships(child, visitor);
    }
}

pub(crate) fn register_branch_memberships<'a>(
    branches: &'a [Branch],
    visitor: &mut impl FnMut(&'a Expression),
) {
    for branch in branches {
        for step in &branch.steps {
            match step {
                Step::Guard { expression }
                | Step::Assign {
                    value: expression, ..
                } => visitor(expression),
                Step::Let { value, .. } => register_memberships(value, visitor),
                Step::Bind { domain, .. } | Step::Choose { domain, .. } => {
                    for expression in domain.children() {
                        visitor(expression);
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Validation {
    variables: usize,
    phase: u8,
    depth: usize,
    locals: usize,
    old: bool,
}
impl Validation {
    fn child(self) -> Self {
        Self {
            depth: self.depth + 1,
            ..self
        }
    }
}

pub(crate) enum Values {
    Range(std::ops::RangeInclusive<i32>),
    Items(std::vec::IntoIter<Scalar>),
}
impl Iterator for Values {
    type Item = Scalar;
    fn next(&mut self) -> Option<Scalar> {
        match self {
            Self::Range(values) => values.next().map(Scalar::Int),
            Self::Items(values) => values.next(),
        }
    }
}

impl Domain {
    fn children(&self) -> Vec<&Expression> {
        match self {
            Self::Range { lower, upper } => vec![lower, upper],
            Self::Set { values } => values.iter().collect(),
            Self::Value { value } => vec![value],
        }
    }
    fn validate(&self, context: Validation) -> Result<(), CoreError> {
        for value in self.children() {
            value.validate_with(context)?;
        }
        Ok(())
    }
    pub(crate) fn values(
        &self,
        ctx: EvalContext<'_>,
        arena: &mut Arena,
    ) -> Result<Values, CoreError> {
        Ok(match self {
            Self::Range { lower, upper } => Values::Range(
                lower.eval(ctx, arena)?.integer()?..=upper.eval(ctx, arena)?.integer()?,
            ),
            Self::Set { values } => {
                let values = values
                    .iter()
                    .map(|value| value.eval(ctx, arena))
                    .collect::<Result<Vec<_>, _>>()?;
                Values::Items(arena.ordered(values)?.into_iter())
            }
            Self::Value { value } => {
                let value = value.eval(ctx, arena)?;
                Values::Items(arena.members(value)?.into_iter())
            }
        })
    }
    pub(crate) fn contains(
        &self,
        value: Scalar,
        ctx: EvalContext<'_>,
        arena: &mut Arena,
    ) -> Result<bool, CoreError> {
        match self {
            Self::Range { lower, upper } => {
                let lower = lower.eval(ctx, arena)?.integer()?;
                let upper = upper.eval(ctx, arena)?.integer()?;
                if lower > upper {
                    return Ok(false);
                }
                let value = value.integer()?;
                Ok(lower <= value && value <= upper)
            }
            Self::Set { .. } => {
                let values = self.values(ctx, arena)?;
                for candidate in values {
                    if arena.equal(value, candidate)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Value { value: set } => {
                let set = set.eval(ctx, arena)?;
                arena.contains(value, set)
            }
        }
    }
}

pub(crate) fn validate_branches(
    branches: &[Branch],
    variables: usize,
    phase: u8,
    locals: usize,
) -> Result<(), CoreError> {
    for branch in branches {
        if branch.steps.len() > 256 {
            return Err(CoreError::protocol("Safety branch exceeds 256 steps"));
        }
        let mut context = Validation {
            variables,
            phase,
            depth: 0,
            locals,
            old: false,
        };
        for step in &branch.steps {
            match step {
                Step::Guard { expression } => expression.validate_with(context)?,
                Step::Assign { slot, value } => {
                    if *slot >= variables {
                        return Err(CoreError::protocol("Invalid safety assignment slot"));
                    }
                    value.validate_with(context)?;
                }
                Step::Choose { slot, domain } => {
                    if *slot >= variables {
                        return Err(CoreError::protocol("Invalid safety assignment slot"));
                    }
                    domain.validate(context)?;
                }
                Step::Let { slot, value, .. } => {
                    if *slot != context.locals {
                        return Err(CoreError::protocol("Invalid let variable slot"));
                    }
                    value.validate_with(context)?;
                    context.locals += 1;
                }
                Step::Bind {
                    slot,
                    domain,
                    tuple_size,
                } => {
                    if *slot != context.locals || tuple_size.is_some_and(|size| size > 256) {
                        return Err(CoreError::protocol("Invalid bound variable slot"));
                    }
                    domain.validate(context)?;
                    context.locals += 1;
                }
            }
        }
    }
    Ok(())
}

impl Expression {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn is_scalar_cost(&self) -> bool {
        let simple = match self {
            Self::Literal { value } => !matches!(value, Scalar::Ref(_)),
            Self::Read { .. }
            | Self::Bound { .. }
            | Self::Old
            | Self::Not { .. }
            | Self::Neg { .. }
            | Self::Add { .. }
            | Self::Sub { .. }
            | Self::Mul { .. }
            | Self::Eq { .. }
            | Self::Ne { .. }
            | Self::Lt { .. }
            | Self::Le { .. }
            | Self::Gt { .. }
            | Self::Ge { .. }
            | Self::And { .. }
            | Self::Or { .. }
            | Self::Implies { .. }
            | Self::If { .. } => true,
            Self::In { domain, .. } => match domain {
                Domain::Range { .. } => true,
                Domain::Set { values } => values.len() <= 16,
                Domain::Value { .. } => false,
            },
            _ => false,
        };
        simple && self.children().into_iter().all(Self::is_scalar_cost)
    }
    fn children(&self) -> Vec<&Self> {
        match self {
            Self::Literal { .. }
            | Self::Data { .. }
            | Self::Read { .. }
            | Self::Bound { .. }
            | Self::Old => vec![],
            Self::Prime { argument }
            | Self::Not { argument }
            | Self::Neg { argument }
            | Self::Domain { argument }
            | Self::Cardinality { argument }
            | Self::SequenceSet { argument }
            | Self::Len { argument }
            | Self::Head { argument }
            | Self::Tail { argument }
            | Self::Powerset { argument }
            | Self::Union { argument } => vec![argument],
            Self::Add { left, right }
            | Self::Sub { left, right }
            | Self::Mul { left, right }
            | Self::Eq { left, right }
            | Self::Ne { left, right }
            | Self::Lt { left, right }
            | Self::Le { left, right }
            | Self::Gt { left, right }
            | Self::Ge { left, right }
            | Self::And { left, right }
            | Self::Or { left, right }
            | Self::Implies { left, right }
            | Self::Cup { left, right }
            | Self::Cap { left, right }
            | Self::Difference { left, right }
            | Self::SubsetEq { left, right }
            | Self::Append { left, right }
            | Self::Concat { left, right }
            | Self::FunctionSet { left, right }
            | Self::FunctionIn { left, right }
            | Self::SequenceIn { left, right } => vec![left, right],
            Self::If {
                condition,
                consequent,
                alternative,
            } => vec![condition, consequent, alternative],
            Self::In { value, domain } => {
                let mut children = vec![value.as_ref()];
                children.extend(domain.children());
                children
            }
            Self::Set { values } | Self::Tuple { values } => values.iter().collect(),
            Self::Record { fields } | Self::RecordSet { fields } => {
                fields.iter().map(|entry| &entry.1).collect()
            }
            Self::At { target, key } => vec![target, key],
            Self::Update { target, updates } => {
                let mut children = vec![target.as_ref()];
                for update in updates {
                    children.extend(update.path.iter());
                    children.push(&update.value);
                }
                children
            }
            Self::Forall { bounds, body }
            | Self::Exists { bounds, body }
            | Self::Function { bounds, body }
            | Self::Filter { bounds, body }
            | Self::MapSet { bounds, body } => {
                let mut children = vec![];
                for bound in bounds {
                    children.extend(bound.domain.children());
                }
                children.push(body);
                children
            }
            Self::Fold {
                initial,
                collection,
                body,
                ..
            } => vec![initial, collection, body],
            Self::Let { value, body, .. } => vec![value, body],
            Self::Call { args, .. } => args.iter().collect(),
            Self::Subseq { sequence, from, to } => vec![sequence, from, to],
            Self::Enabled { .. } => vec![],
        }
    }
    fn is_context_free(&self) -> bool {
        !matches!(
            self,
            Self::Read { .. }
                | Self::Enabled { .. }
                | Self::Bound { .. }
                | Self::Old
                | Self::Prime { .. }
                | Self::Call { .. }
                | Self::Literal {
                    value: Scalar::Ref(_)
                }
        ) && self.children().iter().all(|child| child.is_context_free())
    }
    pub(crate) fn is_constant(&self) -> bool {
        !matches!(
            self,
            Self::Read { .. } | Self::Enabled { .. } | Self::Call { .. }
        ) && self.children().iter().all(|value| value.is_constant())
    }
    pub(crate) fn validate_with_locals(
        &self,
        variables: usize,
        phase: u8,
        locals: usize,
    ) -> Result<(), CoreError> {
        self.validate_with(Validation {
            variables,
            phase,
            depth: 0,
            locals,
            old: false,
        })
    }
    pub(crate) fn validate(
        &self,
        variables: usize,
        phase: u8,
        depth: usize,
    ) -> Result<(), CoreError> {
        self.validate_with(Validation {
            variables,
            phase,
            depth,
            locals: 0,
            old: false,
        })
    }
    fn validate_with(&self, ctx: Validation) -> Result<(), CoreError> {
        if ctx.depth > 1000 {
            return Err(CoreError::protocol(
                "Safety expression nesting exceeds 1000",
            ));
        }
        match self {
            Self::Prime { argument } => {
                if ctx.phase != 1 {
                    return Err(CoreError::protocol("Prime requires an action context"));
                }
                argument.validate_with(Validation {
                    phase: 2,
                    ..ctx.child()
                })?;
                return Ok(());
            }
            Self::Read { slot, next } => {
                if *slot >= ctx.variables
                    || (ctx.phase == 0 && !*next)
                    || (ctx.phase == 2 && *next)
                    || ctx.phase == 3
                {
                    return Err(CoreError::protocol("Invalid safety variable read"));
                }
            }
            Self::Bound { slot } => {
                if *slot >= ctx.locals {
                    return Err(CoreError::protocol("Unbound quantified variable"));
                }
            }
            Self::Old => {
                if !ctx.old {
                    return Err(CoreError::protocol("Unbound EXCEPT @"));
                }
            }
            Self::Forall { bounds, body }
            | Self::Exists { bounds, body }
            | Self::Function { bounds, body }
            | Self::Filter { bounds, body }
            | Self::MapSet { bounds, body } => {
                if bounds.is_empty() || bounds.len() > 40 {
                    return Err(CoreError::protocol("Empty binding list"));
                }
                let mut child = ctx.child();
                for bound in bounds {
                    if bound.slot != child.locals || bound.tuple_size.is_some_and(|size| size > 256)
                    {
                        return Err(CoreError::protocol("Invalid bound variable slot"));
                    }
                    bound.domain.validate(child)?;
                    child.locals += 1;
                }
                body.validate_with(child)?;
                return Ok(());
            }
            Self::Let { slot, value, body } => {
                if *slot != ctx.locals {
                    return Err(CoreError::protocol("Invalid let variable slot"));
                }
                value.validate_with(ctx.child())?;
                let mut child = ctx.child();
                child.locals += 1;
                body.validate_with(child)?;
                return Ok(());
            }
            Self::Fold {
                accumulator,
                initial,
                collection,
                body,
                ..
            } => {
                if *accumulator != ctx.locals {
                    return Err(CoreError::protocol("Invalid fold variable slot"));
                }
                initial.validate_with(ctx.child())?;
                collection.validate_with(ctx.child())?;
                let mut child = ctx.child();
                child.locals += 2;
                body.validate_with(child)?;
                return Ok(());
            }
            Self::Update { target, updates } => {
                target.validate_with(ctx.child())?;
                for update in updates {
                    if update.path.is_empty() || update.path.len() > 48 {
                        return Err(CoreError::protocol("Empty EXCEPT path"));
                    }
                    for part in &update.path {
                        part.validate_with(ctx.child())?;
                    }
                    update.value.validate_with(Validation {
                        old: true,
                        ..ctx.child()
                    })?;
                }
                return Ok(());
            }
            Self::Enabled {
                branches,
                subscript,
            } => {
                if ctx.phase == 3 {
                    return Err(CoreError::protocol("ENABLED is not a constant"));
                }
                validate_branches(branches, ctx.variables, 1, ctx.locals)?;
                if let Some(subscript) = subscript {
                    subscript.validate_with(Validation {
                        phase: 2,
                        ..ctx.child()
                    })?;
                }
                return Ok(());
            }
            _ => {}
        }
        for child in self.children() {
            child.validate_with(ctx.child())?;
        }
        Ok(())
    }
    #[inline]
    pub(crate) fn eval(
        &self,
        ctx: EvalContext<'_>,
        arena: &mut Arena,
    ) -> Result<Scalar, CoreError> {
        match self {
            Self::Literal { value } => Ok(*value),
            Self::Read { slot, next } => ctx.read(*slot, *next),
            Self::Bound { slot } => {
                let mut frame = ctx.lazy;
                while let Some(current) = frame {
                    if current.slot == *slot {
                        return current.force(ctx.next, arena);
                    }
                    frame = current.parent;
                }
                ctx.locals
                    .get(*slot)
                    .copied()
                    .ok_or_else(|| CoreError::protocol("Unbound quantified variable"))
            }
            Self::Old => ctx
                .old
                .ok_or_else(|| CoreError::protocol("Unbound EXCEPT @")),
            _ => self.eval_operator(ctx, arena),
        }
    }
    #[inline(never)]
    fn eval_operator(&self, ctx: EvalContext<'_>, arena: &mut Arena) -> Result<Scalar, CoreError> {
        Ok(match self {
            Self::Literal { .. } | Self::Read { .. } | Self::Bound { .. } | Self::Old => {
                return self.eval(ctx, arena);
            }
            Self::Data { value } => arena.data(value)?,
            Self::Prime { argument } => argument.eval(EvalContext { prime: true, ..ctx }, arena)?,
            Self::Not { argument } => Scalar::Bool(!argument.eval(ctx, arena)?.boolean()?),
            Self::Neg { argument } => Scalar::Int(
                argument
                    .eval(ctx, arena)?
                    .integer()?
                    .checked_neg()
                    .ok_or_else(|| CoreError::protocol("Safety integer overflow"))?,
            ),
            Self::Add { left, right } | Self::Sub { left, right } | Self::Mul { left, right } => {
                let left = left.eval(ctx, arena)?.integer()?;
                let right = right.eval(ctx, arena)?.integer()?;
                Scalar::Int(
                    match self {
                        Self::Add { .. } => left.checked_add(right),
                        Self::Sub { .. } => left.checked_sub(right),
                        _ => left.checked_mul(right),
                    }
                    .ok_or_else(|| CoreError::protocol("Safety integer overflow"))?,
                )
            }
            Self::Eq { left, right } | Self::Ne { left, right } => {
                let left = left.eval(ctx, arena)?;
                let right = right.eval(ctx, arena)?;
                Scalar::Bool(arena.equal(left, right)? == matches!(self, Self::Eq { .. }))
            }
            Self::Lt { left, right }
            | Self::Le { left, right }
            | Self::Gt { left, right }
            | Self::Ge { left, right } => {
                let left = left.eval(ctx, arena)?.integer()?;
                let right = right.eval(ctx, arena)?.integer()?;
                Scalar::Bool(match self {
                    Self::Lt { .. } => left < right,
                    Self::Le { .. } => left <= right,
                    Self::Gt { .. } => left > right,
                    _ => left >= right,
                })
            }
            Self::And { left, right } => Scalar::Bool(
                left.eval(ctx, arena)?.boolean()? && right.eval(ctx, arena)?.boolean()?,
            ),
            Self::Or { left, right } => Scalar::Bool(
                left.eval(ctx, arena)?.boolean()? || right.eval(ctx, arena)?.boolean()?,
            ),
            Self::Implies { left, right } => Scalar::Bool(
                !left.eval(ctx, arena)?.boolean()? || right.eval(ctx, arena)?.boolean()?,
            ),
            Self::If {
                condition,
                consequent,
                alternative,
            } => if condition.eval(ctx, arena)?.boolean()? {
                consequent
            } else {
                alternative
            }
            .eval(ctx, arena)?,
            Self::In { value, domain } => {
                let value = value.eval(ctx, arena)?;
                Scalar::Bool(domain.contains(value, ctx, arena)?)
            }
            Self::Set { values } | Self::Tuple { values } => {
                let mut result = vec![];
                for value in values {
                    arena.tick()?;
                    result.push(value.eval(ctx, arena)?);
                }
                if matches!(self, Self::Set { .. }) {
                    arena.set(result)?
                } else {
                    arena.tuple(result)?
                }
            }
            Self::Record { fields } => {
                let mut result = vec![];
                for (name, value) in fields {
                    arena.tick()?;
                    let name = arena.string(name.clone())?;
                    let value = value.eval(ctx, arena)?;
                    result.push((name, value));
                }
                arena.function(result)?
            }
            Self::At { target, key } => {
                let target = target.eval(ctx, arena)?;
                let key = key.eval(ctx, arena)?;
                arena.at(target, key)?
            }
            Self::Update { target, updates } => {
                let mut result = target.eval(ctx, arena)?;
                for update in updates {
                    arena.tick()?;
                    let path = update
                        .path
                        .iter()
                        .map(|part| part.eval(ctx, arena))
                        .collect::<Result<Vec<_>, _>>()?;
                    if let Some(old) = arena.path(result, &path)? {
                        let value = update.value.eval(
                            EvalContext {
                                old: Some(old),
                                ..ctx
                            },
                            arena,
                        )?;
                        result = arena.replace(result, &path, value)?;
                    }
                }
                result
            }
            Self::Forall { bounds, body } | Self::Exists { bounds, body } => {
                let forall = matches!(self, Self::Forall { .. });
                let mut result = forall;
                visit_bindings(bounds, 0, ctx, arena, &mut |ctx, arena| {
                    let value = body.eval(ctx, arena)?.boolean()?;
                    if value != forall {
                        result = value;
                        Ok(false)
                    } else {
                        Ok(true)
                    }
                })?;
                Scalar::Bool(result)
            }
            Self::Function { bounds, body }
            | Self::Filter { bounds, body }
            | Self::MapSet { bounds, body } => {
                let mut values = vec![];
                let mut entries = vec![];
                visit_bindings(bounds, 0, ctx, arena, &mut |inner, arena| {
                    let value = body.eval(inner, arena)?;
                    if matches!(self, Self::Function { .. }) {
                        let keys = inner.locals[ctx.locals.len()..].to_vec();
                        let key = if keys.len() == 1 {
                            keys[0]
                        } else {
                            arena.tuple(keys)?
                        };
                        entries.push((key, value));
                    } else if matches!(self, Self::Filter { .. }) {
                        if value.boolean()? {
                            if bounds.len() != 1 {
                                return Err(CoreError::protocol("Filter requires one bound"));
                            }
                            values.push(inner.locals[ctx.locals.len()]);
                        }
                    } else {
                        values.push(value);
                    }
                    Ok(true)
                })?;
                if matches!(self, Self::Function { .. }) {
                    arena.function(entries)?
                } else {
                    arena.set(values)?
                }
            }
            Self::Let { slot, value, body } => {
                if *slot != ctx.locals.len() {
                    return Err(CoreError::protocol("Invalid let variable slot"));
                }
                let frame = LazyFrame {
                    slot: *slot,
                    value,
                    ctx,
                    cache: std::cell::Cell::new(None),
                    parent: ctx.lazy,
                    stable: true,
                };
                let mut locals = ctx.locals.to_vec();
                locals.push(Scalar::Bool(false));
                body.eval(
                    EvalContext {
                        locals: &locals,
                        lazy: Some(&frame),
                        ..ctx
                    },
                    arena,
                )?
            }
            Self::Call { operator, args } => {
                let operators = arena.operators;
                let definition = operators
                    .get(*operator)
                    .ok_or_else(|| CoreError::protocol("Unknown operator"))?;
                if definition.arity != args.len() {
                    return Err(CoreError::protocol("Operator arity mismatch"));
                }
                arena.tick()?;
                let locals = vec![Scalar::Bool(false); args.len()];
                bind_arguments(0, args, ctx, None, &definition.body, &locals, arena)?
            }
            Self::Fold {
                accumulator,
                initial,
                collection,
                order,
                body,
            } => {
                if *accumulator != ctx.locals.len() {
                    return Err(CoreError::protocol("Invalid fold variable slot"));
                }
                let mut result = initial.eval(ctx, arena)?;
                let source = collection.eval(ctx, arena)?;
                let elements = match order {
                    FoldOrder::SetMembers => arena.members(source)?,
                    FoldOrder::SequenceLeft => arena.sequence(source)?,
                };
                let mut locals = ctx.locals.to_vec();
                for element in elements {
                    arena.tick()?;
                    locals.push(result);
                    locals.push(element);
                    result = body.eval(
                        EvalContext {
                            locals: &locals,
                            ..ctx
                        },
                        arena,
                    )?;
                    locals.truncate(ctx.locals.len());
                }
                result
            }
            Self::Domain { argument } => {
                let value = argument.eval(ctx, arena)?;
                arena.domain(value)?
            }
            Self::Cardinality { argument } => {
                let value = argument.eval(ctx, arena)?;
                Scalar::Int(
                    i32::try_from(arena.members(value)?.len())
                        .map_err(|_| CoreError::protocol("Set cardinality overflow"))?,
                )
            }
            Self::Len { argument } | Self::Head { argument } | Self::Tail { argument } => {
                let value = argument.eval(ctx, arena)?;
                if let Some(text) = arena.text(value) {
                    let units = text.encode_utf16().collect::<Vec<_>>();
                    return Ok(match self {
                        Self::Len { .. } => Scalar::Int(
                            i32::try_from(units.len())
                                .map_err(|_| CoreError::protocol("String length overflow"))?,
                        ),
                        Self::Head { .. } => {
                            let first = units
                                .first()
                                .ok_or_else(|| CoreError::protocol("Head of an empty string"))?;
                            arena.string_utf16(&[*first])?
                        }
                        _ => {
                            if units.is_empty() {
                                return Err(CoreError::protocol("Tail of an empty string"));
                            }
                            arena.string_utf16(&units[1..])?
                        }
                    });
                }
                let mut values = arena.sequence(value)?;
                match self {
                    Self::Len { .. } => Scalar::Int(values.len() as i32),
                    Self::Head { .. } => *values
                        .first()
                        .ok_or_else(|| CoreError::protocol("Head of an empty sequence"))?,
                    _ => {
                        if values.is_empty() {
                            return Err(CoreError::protocol("Tail of an empty sequence"));
                        }
                        values.remove(0);
                        arena.tuple(values)?
                    }
                }
            }
            Self::Append { left, right } | Self::Concat { left, right } => {
                let left = left.eval(ctx, arena)?;
                let right = right.eval(ctx, arena)?;
                if let Some(text) = arena.text(left) {
                    let suffix = arena.text(right).ok_or_else(|| {
                        CoreError::protocol("String concatenation requires strings")
                    })?;
                    let size = text
                        .len()
                        .checked_add(suffix.len())
                        .ok_or_else(|| CoreError::protocol("String length overflow"))?;
                    arena.consume(size)?;
                    let text = arena
                        .text(left)
                        .ok_or_else(|| CoreError::protocol("Expected a string"))?;
                    let suffix = arena
                        .text(right)
                        .ok_or_else(|| CoreError::protocol("Expected a string"))?;
                    let value = format!("{text}{suffix}");
                    return arena.string(value);
                }
                let mut values = arena.sequence_values(left)?;
                if matches!(self, Self::Append { .. }) {
                    values.push(right);
                } else {
                    values.extend(arena.sequence_values(right)?);
                }
                arena.tuple(values)?
            }
            Self::Subseq { sequence, from, to } => {
                let sequence = sequence.eval(ctx, arena)?;
                let from = from.eval(ctx, arena)?.integer()?;
                let to = to.eval(ctx, arena)?.integer()?;
                if let Some(text) = arena.text(sequence) {
                    let units = text.encode_utf16().collect::<Vec<_>>();
                    if from > to {
                        return arena.string(String::new());
                    }
                    if from < 1 || to as usize > units.len() {
                        return Err(CoreError::protocol("SubSeq bounds are outside the string"));
                    }
                    return arena.string_utf16(&units[(from - 1) as usize..to as usize]);
                }
                let values = arena.sequence(sequence)?;
                if from > to {
                    arena.tuple(vec![])?
                } else {
                    if from < 1 || to as usize > values.len() {
                        return Err(CoreError::protocol(
                            "SubSeq bounds are outside the sequence",
                        ));
                    }
                    arena.tuple(values[(from - 1) as usize..to as usize].to_vec())?
                }
            }
            Self::Cup { left, right }
            | Self::Cap { left, right }
            | Self::Difference { left, right }
            | Self::SubsetEq { left, right } => {
                let left = left.eval(ctx, arena)?;
                let right = right.eval(ctx, arena)?;
                let mut result = vec![];
                for value in arena.members(left)? {
                    arena.tick()?;
                    let contains = arena.contains(value, right)?;
                    if matches!(self, Self::SubsetEq { .. }) && !contains {
                        return Ok(Scalar::Bool(false));
                    }
                    if matches!(self, Self::Cup { .. })
                        || (matches!(self, Self::Cap { .. }) && contains)
                        || (matches!(self, Self::Difference { .. }) && !contains)
                    {
                        result.push(value);
                    }
                }
                if matches!(self, Self::SubsetEq { .. }) {
                    Scalar::Bool(true)
                } else {
                    if matches!(self, Self::Cup { .. }) {
                        result.extend(arena.members(right)?);
                    }
                    arena.set(result)?
                }
            }
            Self::Union { argument } => {
                let outer = argument.eval(ctx, arena)?;
                let mut values = vec![];
                for set in arena.members(outer)? {
                    arena.tick()?;
                    values.extend(arena.members(set)?);
                }
                arena.set(values)?
            }
            Self::Powerset { argument } => {
                let set = argument.eval(ctx, arena)?;
                let members = arena.members(set)?;
                let mut sets = vec![arena.set(vec![])?];
                for value in members {
                    let mut additions = vec![];
                    for set in &sets {
                        arena.tick()?;
                        let mut members = arena.members(*set)?;
                        members.push(value);
                        additions.push(arena.set(members)?);
                    }
                    sets.extend(additions);
                }
                arena.set(sets)?
            }
            Self::FunctionSet { left, right } => {
                let domain = left.eval(ctx, arena)?;
                let range = right.eval(ctx, arena)?;
                let keys = arena.members(domain)?;
                let values = arena.members(range)?;
                let mut functions = vec![];
                enumerate_functions(
                    &keys,
                    &vec![values.as_slice(); keys.len()],
                    arena,
                    &mut functions,
                )?;
                arena.set(functions)?
            }
            Self::RecordSet { fields } => {
                let mut keys = vec![];
                let mut ranges = vec![];
                for (name, set) in fields {
                    keys.push(arena.string(name.clone())?);
                    let set = set.eval(ctx, arena)?;
                    ranges.push(arena.members(set)?);
                }
                let mut functions = vec![];
                enumerate_functions(
                    &keys,
                    &ranges.iter().map(Vec::as_slice).collect::<Vec<_>>(),
                    arena,
                    &mut functions,
                )?;
                arena.set(functions)?
            }
            Self::FunctionIn { left, right } => {
                let value = left.eval(ctx, arena)?;
                let set = MembershipSet::eval(right, ctx, arena)?;
                Scalar::Bool(set.contains(value, arena)?)
            }
            Self::SequenceIn { left, right } => {
                let value = left.eval(ctx, arena)?;
                let set = MembershipSet::new(
                    MembershipKind::Sequences(MembershipSet::eval(right, ctx, arena)?),
                    false,
                );
                Scalar::Bool(set.contains(value, arena)?)
            }
            Self::SequenceSet { .. } => {
                return Err(CoreError::protocol("Sequence sets cannot be enumerated"));
            }
            Self::Enabled {
                branches,
                subscript,
            } => {
                let current = if ctx.current.is_empty() || ctx.prime {
                    ctx.next
                        .iter()
                        .map(|value| {
                            value.ok_or_else(|| {
                                CoreError::protocol("ENABLED requires assigned initial variables")
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    ctx.current.to_vec()
                };
                let mut enabled = false;
                for branch in branches {
                    let mut next = vec![None; current.len()];
                    visit_branch(
                        branch,
                        0,
                        EvalContext {
                            current: &current,
                            next: &[],
                            locals: ctx.locals,
                            old: None,
                            prime: false,
                            lazy: ctx.lazy,
                        },
                        &mut next,
                        arena,
                        &mut |next, arena| {
                            if let Some(subscript) = subscript {
                                let context = EvalContext {
                                    locals: ctx.locals,
                                    ..EvalContext::new(&current, next)
                                };
                                let before = subscript.eval(context, arena)?;
                                let after = subscript.eval(
                                    EvalContext {
                                        prime: true,
                                        ..context
                                    },
                                    arena,
                                )?;
                                if arena.equal(before, after)? {
                                    return Ok(false);
                                }
                            }
                            enabled = true;
                            Ok(true)
                        },
                    )?;
                    if enabled {
                        break;
                    }
                }
                Scalar::Bool(enabled)
            }
        })
    }
}

#[derive(PartialEq, Eq)]
pub(crate) struct MembershipSet {
    kind: MembershipKind,
    cacheable: bool,
    hash: u64,
}
#[derive(PartialEq, Eq, Hash)]
enum MembershipKind {
    Finite(Scalar),
    Sequences(Arc<MembershipSet>),
    Functions {
        domain: Scalar,
        range: Arc<MembershipSet>,
    },
    Records(Vec<(Scalar, Arc<MembershipSet>)>),
    Subsets(Scalar),
}
impl Hash for MembershipSet {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
}
impl MembershipSet {
    pub(crate) fn fingerprint(&self) -> u64 {
        self.hash
    }
    fn new(kind: MembershipKind, cacheable: bool) -> Arc<Self> {
        let mut hasher = DefaultHasher::new();
        kind.hash(&mut hasher);
        Arc::new(Self {
            kind,
            cacheable,
            hash: hasher.finish(),
        })
    }
    fn eval(
        expression: &Expression,
        ctx: EvalContext<'_>,
        arena: &mut Arena,
    ) -> Result<Arc<Self>, CoreError> {
        let cached = arena.prepared_membership(expression);
        if let Some(Some(prepared)) = &cached {
            return Ok(Arc::clone(prepared));
        }
        let cacheable = if cached.is_some() {
            false
        } else {
            expression.is_context_free()
        };
        let kind = match expression {
            Expression::SequenceSet { argument } => {
                MembershipKind::Sequences(Self::eval(argument, ctx, arena)?)
            }
            Expression::FunctionSet { left, right } => MembershipKind::Functions {
                domain: left.eval(ctx, arena)?,
                range: Self::eval(right, ctx, arena)?,
            },
            Expression::RecordSet { fields } => {
                let mut values = vec![];
                for (name, set) in fields {
                    let name = arena.string(name.clone())?;
                    values.push((name, Self::eval(set, ctx, arena)?));
                }
                values.sort_by(|left, right| arena.total(left.0, right.0));
                MembershipKind::Records(values)
            }
            Expression::Powerset { argument } => {
                MembershipKind::Subsets(argument.eval(ctx, arena)?)
            }
            _ => MembershipKind::Finite(expression.eval(ctx, arena)?),
        };
        let prepared = Self::new(kind, cacheable);
        if cached.is_none() {
            arena.cache_membership_type(expression, cacheable.then(|| Arc::clone(&prepared)));
        }
        Ok(prepared)
    }
    fn contains(self: &Arc<Self>, value: Scalar, arena: &mut Arena) -> Result<bool, CoreError> {
        let cacheable = self.cacheable && matches!(self.kind, MembershipKind::Records(_));
        if cacheable && let Some(result) = arena.membership_result(self, value) {
            return Ok(result);
        }
        let result = self.contains_uncached(value, arena)?;
        if cacheable {
            arena.cache_membership_result(Arc::clone(self), value, result);
        }
        Ok(result)
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn remap(self: &Arc<Self>, mapping: &ValueRemap) -> Arc<Self> {
        let changed = match &self.kind {
            MembershipKind::Finite(value) | MembershipKind::Subsets(value) => {
                mapping.get(*value) != *value
            }
            MembershipKind::Sequences(set) => !Arc::ptr_eq(set, &set.remap(mapping)),
            MembershipKind::Functions { domain, range } => {
                mapping.get(*domain) != *domain || !Arc::ptr_eq(range, &range.remap(mapping))
            }
            MembershipKind::Records(fields) => fields.iter().any(|(key, set)| {
                mapping.get(*key) != *key || !Arc::ptr_eq(set, &set.remap(mapping))
            }),
        };
        if !changed {
            return Arc::clone(self);
        }
        let kind = match &self.kind {
            MembershipKind::Finite(value) => MembershipKind::Finite(mapping.get(*value)),
            MembershipKind::Subsets(value) => MembershipKind::Subsets(mapping.get(*value)),
            MembershipKind::Sequences(set) => MembershipKind::Sequences(set.remap(mapping)),
            MembershipKind::Functions { domain, range } => MembershipKind::Functions {
                domain: mapping.get(*domain),
                range: range.remap(mapping),
            },
            MembershipKind::Records(fields) => MembershipKind::Records(
                fields
                    .iter()
                    .map(|(key, set)| (mapping.get(*key), set.remap(mapping)))
                    .collect(),
            ),
        };
        Self::new(kind, self.cacheable)
    }
    fn contains_uncached(&self, value: Scalar, arena: &mut Arena) -> Result<bool, CoreError> {
        match &self.kind {
            MembershipKind::Finite(set) => arena.contains(value, *set),
            MembershipKind::Subsets(set) => {
                for element in arena.members(value)? {
                    arena.tick()?;
                    if !arena.contains(element, *set)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            MembershipKind::Sequences(set) => {
                for element in arena.sequence_values(value)? {
                    arena.tick()?;
                    if !set.contains(element, arena)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            MembershipKind::Functions { domain, range } => {
                let entries = arena.entries(value)?;
                if !arena.has_domain(value, *domain)? {
                    return Ok(false);
                }
                for (_, element) in entries {
                    arena.tick()?;
                    if !range.contains(element, arena)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            MembershipKind::Records(fields) => {
                let entries = arena.entries(value)?;
                if entries.iter().any(|(key, _)| arena.text(*key).is_none()) {
                    return Err(CoreError::protocol(
                        "Record membership requires string keys",
                    ));
                }
                if entries.len() != fields.len() {
                    return Ok(false);
                }
                for (key, set) in fields {
                    let Some(element) = arena.select(value, *key)? else {
                        return Ok(false);
                    };
                    arena.tick()?;
                    if !set.contains(element, arena)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }
}

fn visit_bindings(
    bindings: &[Binding],
    position: usize,
    ctx: EvalContext<'_>,
    arena: &mut Arena,
    visitor: &mut impl FnMut(EvalContext<'_>, &mut Arena) -> Result<bool, CoreError>,
) -> Result<bool, CoreError> {
    let Some(binding) = bindings.get(position) else {
        return visitor(ctx, arena);
    };
    let values = binding.domain.values(ctx, arena)?;
    let mut locals = ctx.locals.to_vec();
    if binding.slot != locals.len() {
        return Err(CoreError::protocol("Unbound variable slot"));
    }
    for value in values {
        arena.tick()?;
        arena.check_tuple(value, binding.tuple_size)?;
        locals.push(value);
        let keep = visit_bindings(
            bindings,
            position + 1,
            EvalContext {
                locals: &locals,
                ..ctx
            },
            arena,
            visitor,
        )?;
        locals.pop();
        if !keep {
            return Ok(false);
        }
    }
    Ok(true)
}

fn enumerate_functions(
    keys: &[Scalar],
    ranges: &[&[Scalar]],
    arena: &mut Arena,
    output: &mut Vec<Scalar>,
) -> Result<(), CoreError> {
    if ranges.iter().any(|range| range.is_empty()) {
        return Ok(());
    }
    let mut positions = vec![0; keys.len()];
    loop {
        arena.tick()?;
        let mut entries = Vec::with_capacity(keys.len());
        for (index, key) in keys.iter().enumerate() {
            arena.tick()?;
            entries.push((*key, ranges[index][positions[index]]));
        }
        output.push(arena.function(entries)?);
        let mut index = keys.len();
        loop {
            if index == 0 {
                return Ok(());
            }
            index -= 1;
            positions[index] += 1;
            if positions[index] < ranges[index].len() {
                break;
            }
            positions[index] = 0;
        }
    }
}

pub(crate) fn visit_branch(
    branch: &Branch,
    position: usize,
    ctx: EvalContext<'_>,
    next: &mut [Option<Scalar>],
    arena: &mut Arena,
    visitor: &mut impl FnMut(&[Option<Scalar>], &mut Arena) -> Result<bool, CoreError>,
) -> Result<bool, CoreError> {
    arena.tick()?;
    let Some(step) = branch.steps.get(position) else {
        return visitor(next, arena);
    };
    let context = EvalContext { next, ..ctx };
    match step {
        Step::Guard { expression } => {
            if expression.eval(context, arena)?.boolean()? {
                visit_branch(branch, position + 1, ctx, next, arena, visitor)
            } else {
                Ok(false)
            }
        }
        Step::Assign { slot, value } => {
            let value = value.eval(context, arena)?;
            let previous = next[*slot];
            if let Some(previous) = previous
                && !arena.equal(previous, value)?
            {
                return Ok(false);
            }
            next[*slot] = Some(value);
            let result = visit_branch(branch, position + 1, ctx, next, arena, visitor);
            next[*slot] = previous;
            result
        }
        Step::Choose { slot, domain } => {
            if let Some(value) = next[*slot] {
                return if domain.contains(value, context, arena)? {
                    visit_branch(branch, position + 1, ctx, next, arena, visitor)
                } else {
                    Ok(false)
                };
            }
            for value in domain.values(context, arena)? {
                next[*slot] = Some(value);
                let found = visit_branch(branch, position + 1, ctx, next, arena, visitor)?;
                next[*slot] = None;
                if found {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Step::Let {
            slot,
            value,
            stable,
        } => {
            if *slot != ctx.locals.len() {
                return Err(CoreError::protocol("Unbound action variable"));
            }
            let frame = LazyFrame {
                slot: *slot,
                value,
                ctx: EvalContext { next: &[], ..ctx },
                cache: std::cell::Cell::new(None),
                parent: ctx.lazy,
                stable: *stable,
            };
            let mut locals = ctx.locals.to_vec();
            locals.push(Scalar::Bool(false));
            visit_branch(
                branch,
                position + 1,
                EvalContext {
                    locals: &locals,
                    lazy: Some(&frame),
                    ..ctx
                },
                next,
                arena,
                visitor,
            )
        }
        Step::Bind {
            slot,
            domain,
            tuple_size,
        } => {
            let values = domain.values(context, arena)?;
            let mut locals = ctx.locals.to_vec();
            if *slot != locals.len() {
                return Err(CoreError::protocol("Unbound action variable"));
            }
            for value in values {
                arena.check_tuple(value, *tuple_size)?;
                locals.push(value);
                let found = visit_branch(
                    branch,
                    position + 1,
                    EvalContext {
                        locals: &locals,
                        ..ctx
                    },
                    next,
                    arena,
                    visitor,
                )?;
                locals.pop();
                if found {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EvalContext;
    use crate::Value;
    use crate::ir::{Expression, Scalar};
    use crate::safety_values::Arena;

    fn membership(element: Expression) -> Expression {
        Expression::FunctionIn {
            left: Box::new(Expression::Read {
                slot: 0,
                next: false,
            }),
            right: Box::new(Expression::RecordSet {
                fields: vec![(
                    "value".into(),
                    Expression::Set {
                        values: vec![element],
                    },
                )],
            }),
        }
    }

    #[test]
    fn successful_record_type_checks_reuse_their_work() {
        let mut arena = Arena::new(1000);
        let value = arena
            .data(&Value::Record(vec![(
                "value".into(),
                Value::Int("0".into()),
            )]))
            .unwrap();
        let expression = membership(Expression::Literal {
            value: Scalar::Int(0),
        });
        assert!(
            expression
                .eval(EvalContext::new(&[value], &[]), &mut arena)
                .unwrap()
                .boolean()
                .unwrap()
        );
        let work = arena.work;
        for _ in 0..10 {
            assert!(
                expression
                    .eval(EvalContext::new(&[value], &[]), &mut arena)
                    .unwrap()
                    .boolean()
                    .unwrap()
            );
        }
        assert_eq!(arena.work, work);
    }

    #[test]
    fn cached_type_results_compare_descriptors_after_hash_collisions() {
        use super::{MembershipKind, MembershipSet};
        use std::sync::Arc;
        let mut arena = Arena::new(1000);
        let key = arena.string("value".into()).unwrap();
        let value = arena.function(vec![(key, Scalar::Int(0))]).unwrap();
        let zero = arena.set(vec![Scalar::Int(0)]).unwrap();
        let one = arena.set(vec![Scalar::Int(1)]).unwrap();
        let first = Arc::new(MembershipSet {
            hash: 7,
            cacheable: true,
            kind: MembershipKind::Records(vec![(
                key,
                MembershipSet::new(MembershipKind::Finite(zero), true),
            )]),
        });
        let second = Arc::new(MembershipSet {
            hash: 7,
            cacheable: true,
            kind: MembershipKind::Records(vec![(
                key,
                MembershipSet::new(MembershipKind::Finite(one), true),
            )]),
        });
        arena.cache_membership_result(Arc::clone(&first), value, true);
        arena.cache_membership_result(Arc::clone(&second), value, false);
        assert_eq!(arena.membership_result(&first, value), Some(true));
        assert_eq!(arena.membership_result(&second, value), Some(false));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn cached_membership_values_and_descriptors_remap_with_worker_references() {
        use crate::safety_work::WorkBudget;
        let mut base = Arena::new(1000);
        let budget = WorkBudget::new(900, 2);
        let first = membership(Expression::Literal {
            value: Scalar::Int(1),
        });
        let second = membership(Expression::Literal {
            value: Scalar::Int(2),
        });
        let mut left = base.fork(&budget, 0);
        let mut right = base.fork(&budget, 1);
        let left_value = left
            .data(&Value::Record(vec![(
                "value".into(),
                Value::Int("1".into()),
            )]))
            .unwrap();
        let right_value = right
            .data(&Value::Record(vec![(
                "value".into(),
                Value::Int("2".into()),
            )]))
            .unwrap();
        assert_eq!(left_value, right_value);
        assert!(
            first
                .eval(EvalContext::new(&[left_value], &[]), &mut left)
                .unwrap()
                .boolean()
                .unwrap()
        );
        assert!(
            second
                .eval(EvalContext::new(&[right_value], &[]), &mut right)
                .unwrap()
                .boolean()
                .unwrap()
        );
        let left = left.into_delta();
        let right = right.into_delta();
        let left_map = base.merge(left).unwrap();
        let right_map = base.merge(right).unwrap();
        let left_value = left_map.get(left_value);
        let right_value = right_map.get(right_value);
        assert_ne!(left_value, right_value);
        assert!(
            first
                .eval(EvalContext::new(&[left_value], &[]), &mut base)
                .unwrap()
                .boolean()
                .unwrap()
        );
        assert!(
            second
                .eval(EvalContext::new(&[right_value], &[]), &mut base)
                .unwrap()
                .boolean()
                .unwrap()
        );
        assert!(
            !first
                .eval(EvalContext::new(&[right_value], &[]), &mut base)
                .unwrap()
                .boolean()
                .unwrap()
        );
    }

    #[test]
    fn record_types_depending_on_bound_variables_are_not_reused() {
        let mut arena = Arena::new(1000);
        let value = arena
            .data(&Value::Record(vec![(
                "value".into(),
                Value::Int("0".into()),
            )]))
            .unwrap();
        let expression = membership(Expression::Bound { slot: 0 });
        for (number, expected) in [(0, true), (1, false), (0, true)] {
            let current = [value];
            let locals = [Scalar::Int(number)];
            let context = EvalContext {
                locals: &locals,
                ..EvalContext::new(&current, &[])
            };
            assert_eq!(
                expression
                    .eval(context, &mut arena)
                    .unwrap()
                    .boolean()
                    .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn preparing_types_does_not_evaluate_an_unreached_branch_or_cache_an_error() {
        let mut arena = Arena::new(1000);
        let value = arena
            .data(&Value::Record(vec![(
                "value".into(),
                Value::Int("0".into()),
            )]))
            .unwrap();
        let invalid = membership(Expression::Head {
            argument: Box::new(Expression::Tuple { values: vec![] }),
        });
        let expression = Expression::If {
            condition: Box::new(Expression::Literal {
                value: Scalar::Bool(false),
            }),
            consequent: Box::new(invalid.clone()),
            alternative: Box::new(Expression::Literal {
                value: Scalar::Bool(true),
            }),
        };
        assert!(
            expression
                .eval(EvalContext::new(&[value], &[]), &mut arena)
                .unwrap()
                .boolean()
                .unwrap()
        );
        assert!(
            invalid
                .eval(EvalContext::new(&[value], &[]), &mut arena)
                .is_err()
        );
        assert!(
            invalid
                .eval(EvalContext::new(&[value], &[]), &mut arena)
                .is_err()
        );
    }
    #[test]
    fn leaf_evaluation_preserves_context_errors_and_lazy_branches() {
        let current = [Scalar::Int(3)];
        let next = [Some(Scalar::Int(7))];
        let locals = [Scalar::Bool(true)];
        let context = EvalContext {
            current: &current,
            next: &next,
            locals: &locals,
            old: Some(Scalar::Int(-1)),
            prime: false,
            lazy: None,
        };
        let mut arena = Arena::new(0);
        for (expression, expected) in [
            (
                Expression::Literal {
                    value: Scalar::Bool(false),
                },
                Scalar::Bool(false),
            ),
            (
                Expression::Read {
                    slot: 0,
                    next: false,
                },
                Scalar::Int(3),
            ),
            (
                Expression::Read {
                    slot: 0,
                    next: true,
                },
                Scalar::Int(7),
            ),
            (Expression::Bound { slot: 0 }, Scalar::Bool(true)),
            (Expression::Old, Scalar::Int(-1)),
            (
                Expression::Prime {
                    argument: Box::new(Expression::Read {
                        slot: 0,
                        next: false,
                    }),
                },
                Scalar::Int(7),
            ),
        ] {
            assert_eq!(expression.eval(context, &mut arena).unwrap(), expected);
        }
        let empty = EvalContext::new(&[], &[]);
        for (expression, expected) in [
            (
                Expression::Read {
                    slot: 0,
                    next: false,
                },
                "Read of a missing current variable",
            ),
            (
                Expression::Read {
                    slot: 0,
                    next: true,
                },
                "Read of an unassigned next variable",
            ),
            (Expression::Bound { slot: 0 }, "Unbound quantified variable"),
            (Expression::Old, "Unbound EXCEPT @"),
        ] {
            assert_eq!(
                expression.eval(empty, &mut arena).unwrap_err().message,
                expected
            );
        }
        let expression = Expression::If {
            condition: Box::new(Expression::Literal {
                value: Scalar::Bool(false),
            }),
            consequent: Box::new(Expression::Old),
            alternative: Box::new(Expression::Literal {
                value: Scalar::Int(42),
            }),
        };
        assert_eq!(expression.eval(empty, &mut arena).unwrap(), Scalar::Int(42));
        assert_eq!(arena.work, 0);
    }
}
