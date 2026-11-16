use rtla::ast::{
    self, Config, ConfigValue, Definition, DefinitionKind, Expression, ExpressionKind as E, Module,
    Pattern,
};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileError {
    pub message: String,
}
impl std::fmt::Display for CompileError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(output, "Unsupported TLA input: {}", self.message)
    }
}
impl std::error::Error for CompileError {}
type Result<T> = std::result::Result<T, CompileError>;
fn unsupported<T>(message: impl Into<String>) -> Result<T> {
    Err(CompileError {
        message: message.into(),
    })
}
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Initial,
    Next,
    State,
    Constant,
}
#[derive(Clone)]
struct Context {
    phase: Phase,
    locals: BTreeMap<String, Json>,
    calls: Vec<String>,
    depth: usize,
    bound: usize,
    scope: usize,
}
impl Context {
    fn nested(&self) -> Result<Self> {
        if self.depth >= 1000 {
            return unsupported("expression or operator nesting exceeds 1000");
        }
        Ok(Self {
            depth: self.depth + 1,
            ..self.clone()
        })
    }
}
#[derive(Clone)]
struct Entry {
    definition: Definition,
    locals: BTreeMap<String, Json>,
    scope: usize,
}
struct Compiler<'a> {
    module: &'a Module,
    config: &'a Config,
    slots: BTreeMap<String, usize>,
    constants: BTreeMap<String, ConfigValue>,
    scopes: Vec<BTreeMap<String, Entry>>,
    operators: Vec<Json>,
    operator_index: HashMap<(String, u8), usize>,
    in_progress: HashSet<(String, u8)>,
    action_free: HashMap<String, bool>,
}
fn phase_code(phase: Phase) -> u8 {
    match phase {
        Phase::Initial => 0,
        Phase::Next => 1,
        Phase::State => 2,
        Phase::Constant => 3,
    }
}
fn scan_names(value: &Json, names: &mut Vec<String>, size: &mut usize) -> bool {
    match value {
        Json::Object(map) => {
            if let Some(kind) = map.get("kind").and_then(Json::as_str) {
                *size += 1;
                if [
                    "Prime",
                    "Unchanged",
                    "Enabled",
                    "Always",
                    "Eventually",
                    "BoxAction",
                    "AngleAction",
                    "WeakFairness",
                    "StrongFairness",
                ]
                .contains(&kind)
                    || (kind == "UnboundedQuantifier"
                        && map.get("isTemporal") == Some(&json!(true)))
                {
                    return false;
                }
                if kind == "Identifier"
                    && let Some(name) = map.get("name").and_then(Json::as_str)
                {
                    names.push(name.into());
                }
                if kind == "Apply"
                    && let Some(name) = map.get("operator").and_then(Json::as_str)
                {
                    names.push(name.into());
                }
            }
            map.values().all(|inner| scan_names(inner, names, size))
        }
        Json::Array(values) => values.iter().all(|inner| scan_names(inner, names, size)),
        _ => true,
    }
}
fn boolean(value: bool) -> Json {
    json!({"op":"literal","value":{"type":"bool","value":value}})
}
fn integer(value: i64) -> Result<Json> {
    if i32::try_from(value).is_err() {
        return unsupported("integer outside signed 32 bits");
    }
    Ok(json!({"op":"literal","value":{"type":"int","value":value.to_string()}}))
}
fn string(value: &str) -> Json {
    json!({"op":"data","value":{"type":"string","value":value}})
}
fn literal(value: &ConfigValue) -> Result<Json> {
    match value {
        ConfigValue::Number { value } => integer(*value),
        ConfigValue::Boolean { value } => Ok(boolean(*value)),
        ConfigValue::String { value } => Ok(string(value)),
        ConfigValue::Set { elements } => Ok(
            json!({"op":"set","values":elements.iter().map(literal).collect::<Result<Vec<_>>>()?}),
        ),
        ConfigValue::ModelValue { .. } => unsupported("model values are not strings"),
    }
}
fn level(value: &Json) -> u8 {
    match value {
        Json::Object(fields) => match fields.get("op").and_then(Json::as_str) {
            Some("read") => {
                if value["next"] == true {
                    2
                } else {
                    1
                }
            }
            Some("prime") => 2,
            Some("enabled") => 1,
            _ => fields.values().map(level).max().unwrap_or(0),
        },
        Json::Array(values) => values.iter().map(level).max().unwrap_or(0),
        _ => 0,
    }
}
fn prime(value: Json) -> Result<Json> {
    if level(&value) > 1 {
        return unsupported("prime of an action expression");
    }
    Ok(json!({"op":"prime","argument":value}))
}
fn atom(expression: Json) -> Json {
    json!({"kind":"atom","expression":expression})
}
fn negate(formula: Json) -> Json {
    if formula["kind"] == "atom" {
        atom(json!({"op":"not","argument":formula["expression"]}))
    } else {
        json!({"kind":"not","formula":formula})
    }
}
fn combine(kind: &str, left: Json, right: Json) -> Json {
    if left["kind"] == "atom" && right["kind"] == "atom" {
        atom(json!({"op":kind,"left":left["expression"],"right":right["expression"]}))
    } else {
        json!({"kind":kind,"left":left,"right":right})
    }
}
fn limited(branches: Vec<Json>) -> Result<Vec<Json>> {
    if branches.len() > 4096
        || branches.iter().any(|branch| {
            branch["steps"]
                .as_array()
                .is_none_or(|steps| steps.len() > 256)
        })
    {
        return unsupported("action expansion exceeds 4096 branches or 256 steps");
    }
    Ok(branches)
}
fn steps(branch: &Json) -> Result<&Vec<Json>> {
    branch["steps"].as_array().ok_or_else(|| CompileError {
        message: "invalid internal action".into(),
    })
}
fn unchanged(value: Json) -> Result<Vec<Json>> {
    if value["op"] == "tuple" {
        let mut result = vec![];
        for item in value["values"].as_array().into_iter().flatten() {
            result.extend(unchanged(item.clone())?);
        }
        return Ok(result);
    }
    if value["op"] != "read" || value["next"] != false {
        return unsupported("generative UNCHANGED requires state variables");
    }
    Ok(vec![
        json!({"kind":"assign","slot":value["slot"],"value":value}),
    ])
}

/// Compile the selected finite TLA+ fragment. Parsing and execution remain separate.
pub fn compile(module: &Module, config: &Config) -> Result<rtlc::SafetyProgram> {
    if module
        .extends
        .iter()
        .any(|name| !["Naturals", "Integers", "Sequences", "FiniteSets"].contains(&name.as_str()))
    {
        return unsupported("EXTENDS");
    }
    if !config.constraints.is_empty()
        || !config.action_constraints.is_empty()
        || !config.substitutions.is_empty()
        || config.symmetry.is_some()
        || config.view.is_some()
        || config.alias.is_some()
        || !config.postconditions.is_empty()
        || !config.possible.is_empty()
        || config.periodic.is_some()
        || config.rl_reward.is_some()
    {
        return unsupported("constrained configuration");
    }
    if if config.specification.is_some() {
        config.init.is_some() || config.next.is_some()
    } else {
        config.init.is_none() || config.next.is_none()
    } {
        return unsupported("use SPECIFICATION or INIT / NEXT");
    }
    if module.variables.is_empty()
        || module.variables.len() > 256
        || module.variables.iter().any(|value| value.arity != 0)
    {
        return unsupported("state variables");
    }
    let symbols: Vec<_> = module
        .variables
        .iter()
        .map(|value| &value.name)
        .chain(module.constants.iter().map(|value| &value.name))
        .chain(module.definitions.iter().map(|value| &value.name))
        .collect();
    if symbols.iter().collect::<HashSet<_>>().len() != symbols.len()
        || module
            .definitions
            .iter()
            .any(|definition| match &definition.head {
                DefinitionKind::Operator { params } => {
                    params.iter().any(|param| symbols.contains(&&param.name))
                        || params
                            .iter()
                            .map(|param| &param.name)
                            .collect::<HashSet<_>>()
                            .len()
                            != params.len()
                }
                _ => false,
            })
    {
        return unsupported("conflicting symbols");
    }
    let mut constants = BTreeMap::new();
    for assignment in &config.constants {
        if assignment.module.is_some()
            || assignment.args.is_some()
            || constants
                .insert(assignment.name.clone(), assignment.value.clone())
                .is_some()
        {
            return unsupported("constant assignment");
        }
    }
    if module
        .constants
        .iter()
        .any(|value| value.arity != 0 || !constants.contains_key(&value.name))
        || constants
            .keys()
            .any(|name| !module.constants.iter().any(|value| &value.name == name))
    {
        return unsupported("missing or extra constants");
    }
    let entries = module
        .definitions
        .iter()
        .map(|definition| {
            (
                definition.name.clone(),
                Entry {
                    definition: definition.clone(),
                    locals: BTreeMap::new(),
                    scope: 0,
                },
            )
        })
        .collect();
    let mut compiler = Compiler {
        module,
        config,
        constants,
        slots: module
            .variables
            .iter()
            .enumerate()
            .map(|(slot, value)| (value.name.clone(), slot))
            .collect(),
        scopes: vec![entries],
        operators: vec![],
        operator_index: HashMap::new(),
        in_progress: HashSet::new(),
        action_free: HashMap::new(),
    };
    let value = compiler.program()?;
    serde_json::from_value(value).map_err(|error| CompileError {
        message: format!("invalid generated IR: {error}"),
    })
}

impl Compiler<'_> {
    fn root(&self, phase: Phase) -> Context {
        Context {
            phase,
            locals: BTreeMap::new(),
            calls: vec![],
            depth: 0,
            bound: 0,
            scope: 0,
        }
    }
    fn has(&self, context: &Context, name: &str) -> bool {
        self.scopes[context.scope].contains_key(name)
    }
    fn extends(&self, name: &str) -> bool {
        self.module.extends.iter().any(|item| item == name)
    }
    fn arithmetic(&self) -> bool {
        self.module.extends.iter().any(|name| {
            ["Naturals", "Integers", "Sequences", "FiniteSets"].contains(&name.as_str())
        })
    }
    fn top_level_action_free(&mut self, name: &str) -> (bool, usize) {
        let Some(entry) = self.scopes[0].get(name) else {
            return (true, 0);
        };
        let definition = entry.definition.clone();
        let tree = serde_json::to_value(&definition.body).unwrap_or(Json::Null);
        let mut names = vec![];
        let mut size = 0;
        if !scan_names(&tree, &mut names, &mut size) {
            return (false, size);
        }
        if let Some(result) = self.action_free.get(name) {
            return (*result, size);
        }
        self.action_free.insert(name.into(), true);
        for referenced in names {
            if referenced != name
                && self.scopes[0].contains_key(&referenced)
                && !self.top_level_action_free(&referenced).0
            {
                self.action_free.insert(name.into(), false);
                return (false, size);
            }
        }
        (true, size)
    }
    fn shareable(&mut self, name: &str, context: &Context) -> bool {
        if context.locals.contains_key(name) {
            return false;
        }
        let Some(entry) = self.scopes[context.scope].get(name) else {
            return false;
        };
        if entry.scope != 0 || entry.definition.is_recursive {
            return false;
        }
        let DefinitionKind::Operator { params } = &entry.definition.head else {
            return false;
        };
        if params.iter().any(|param| param.arity != 0) {
            return false;
        }
        let (free, size) = self.top_level_action_free(name);
        free && size >= 12
    }
    fn shared_call(
        &mut self,
        name: &str,
        args: &[Expression],
        context: &Context,
        child: &Context,
    ) -> Result<Json> {
        let index = self.shared_operator(name, context.phase)?;
        let args = self.expressions(args, child)?;
        Ok(json!({"op":"call","operator":index,"args":args}))
    }
    fn shared_operator(&mut self, name: &str, phase: Phase) -> Result<usize> {
        let key = (name.to_string(), phase_code(phase));
        if let Some(index) = self.operator_index.get(&key) {
            return Ok(*index);
        }
        if !self.in_progress.insert(key.clone()) {
            return unsupported(format!("operator {name}"));
        }
        let entry = self.scopes[0]
            .get(name)
            .cloned()
            .ok_or_else(|| CompileError {
                message: format!("operator {name}"),
            })?;
        let DefinitionKind::Operator { params } = &entry.definition.head else {
            return unsupported(format!("operator {name}"));
        };
        let locals = params
            .iter()
            .enumerate()
            .map(|(slot, param)| (param.name.clone(), json!({"op":"bound","slot":slot})))
            .collect();
        let context = Context {
            phase,
            locals,
            calls: vec![name.into()],
            depth: 0,
            bound: params.len(),
            scope: 0,
        };
        let body = self.expression(&entry.definition.body, &context)?;
        self.in_progress.remove(&key);
        let index = self.operators.len();
        self.operators
            .push(json!({"name":name,"arity":params.len(),"phase":phase_code(phase),"body":body}));
        self.operator_index.insert(key, index);
        Ok(index)
    }
    fn call(
        &mut self,
        name: &str,
        args: &[Expression],
        context: &Context,
    ) -> Result<(Expression, Context)> {
        let entry = self.scopes[context.scope]
            .get(name)
            .cloned()
            .ok_or_else(|| CompileError {
                message: format!("operator {name}"),
            })?;
        if entry.definition.is_recursive || context.calls.iter().any(|item| item == name) {
            return unsupported(format!("operator {name}"));
        }
        let mut locals = entry.locals;
        let body = match &entry.definition.head {
            DefinitionKind::Operator { params } => {
                if params.len() != args.len() || params.iter().any(|param| param.arity != 0) {
                    return unsupported(format!("operator {name}"));
                }
                for (param, argument) in params.iter().zip(args) {
                    locals.insert(param.name.clone(), self.expression(argument, context)?);
                }
                entry.definition.body.clone()
            }
            DefinitionKind::Function { bounds } => {
                if !args.is_empty() {
                    return unsupported(format!("function operator {name}"));
                }
                Expression {
                    node: E::FunctionConstruction {
                        bounds: bounds.clone(),
                        body: Box::new(entry.definition.body.clone()),
                    },
                    location: entry.definition.location,
                }
            }
        };
        let mut child = context.nested()?;
        child.locals = locals;
        child.scope = entry.scope;
        child.calls.push(name.into());
        Ok((body, child))
    }
    fn let_context(&mut self, definitions: &[Definition], context: &Context) -> Result<Context> {
        if definitions
            .iter()
            .map(|definition| &definition.name)
            .collect::<HashSet<_>>()
            .len()
            != definitions.len()
        {
            return unsupported("duplicate LET definitions");
        }
        let scope = self.scopes.len();
        let mut entries = self.scopes[context.scope].clone();
        for definition in definitions {
            entries.insert(
                definition.name.clone(),
                Entry {
                    definition: definition.clone(),
                    locals: context.locals.clone(),
                    scope,
                },
            );
        }
        self.scopes.push(entries);
        Ok(Context {
            scope,
            ..context.nested()?
        })
    }
    fn bind(&mut self, bounds: &[ast::Bound], context: &Context) -> Result<(Vec<Json>, Context)> {
        let mut child = context.clone();
        let mut bindings = vec![];
        for bound in bounds {
            if child.bound >= 1000 {
                return unsupported("more than 1000 bound variables");
            }
            let slot = child.bound;
            let mut binding = json!({"slot":slot,"domain":self.domain(&bound.set,context,true)?});
            match &bound.pattern {
                Pattern::Name { name } => {
                    child
                        .locals
                        .insert(name.clone(), json!({"op":"bound","slot":slot}));
                }
                Pattern::Tuple { names } => {
                    binding["tupleSize"] = json!(names.len());
                    for (index, name) in names.iter().enumerate() {
                        child.locals.insert(name.clone(),json!({"op":"at","target":{"op":"bound","slot":slot},"key":integer(index as i64 + 1)?}));
                    }
                }
            }
            child = child.nested()?;
            child.bound = slot + 1;
            bindings.push(binding);
        }
        Ok((bindings, child))
    }
    fn domain(&mut self, value: &Expression, context: &Context, choice: bool) -> Result<Json> {
        match &value.node {
            E::SetEnumeration { elements } => {
                Ok(json!({"kind":"set","values":self.expressions(elements,&context.nested()?)?}))
            }
            E::Apply { operator, args } if operator == ".." && args.len() == 2 => {
                if !self.arithmetic() {
                    return unsupported("range requires Naturals");
                }
                let child = context.nested()?;
                Ok(
                    json!({"kind":"range","lower":self.expression(&args[0],&child)?,"upper":self.expression(&args[1],&child)?}),
                )
            }
            E::Identifier { name } if name == "BOOLEAN" => {
                Ok(json!({"kind":"set","values":[boolean(false),boolean(true)]}))
            }
            E::Identifier { name } if name == "Nat" || name == "Int" => {
                if choice || !self.arithmetic() || (name == "Int" && !self.extends("Integers")) {
                    return unsupported("infinite enumeration");
                }
                Ok(
                    json!({"kind":"range","lower":integer(if name == "Nat" {0} else {i32::MIN as i64})?,"upper":integer(i32::MAX as i64)?}),
                )
            }
            E::Identifier { name }
                if self.has(context, name) && !context.locals.contains_key(name) =>
            {
                let (body, child) = self.call(name, &[], context)?;
                self.domain(&body, &child, choice)
            }
            _ => Ok(json!({"kind":"value","value":self.expression(value,context)?})),
        }
    }
    fn expressions(&mut self, values: &[Expression], context: &Context) -> Result<Vec<Json>> {
        values
            .iter()
            .map(|value| self.expression(value, context))
            .collect()
    }
    fn expression(&mut self, value: &Expression, context: &Context) -> Result<Json> {
        let child = context.nested()?;
        match &value.node {
            E::Number { value } => integer(i64::try_from(*value).map_err(|_| CompileError {
                message: "integer outside signed 32 bits".into(),
            })?),
            E::String { value } => {
                if value.chars().any(|character| character as u32 > 255) {
                    return unsupported("TLC 1.7.4 string literal outside byte range");
                }
                Ok(string(value))
            }
            E::Identifier { name } => {
                if let Some(local) = context.locals.get(name) {
                    return Ok(local.clone());
                }
                if let Some(constant) = self.constants.get(name) {
                    return literal(constant);
                }
                if name == "TRUE" || name == "FALSE" {
                    return Ok(boolean(name == "TRUE"));
                }
                if name == "BOOLEAN" {
                    return Ok(json!({"op":"set","values":[boolean(false),boolean(true)]}));
                }
                if let Some(slot) = self.slots.get(name) {
                    if context.phase == Phase::Constant {
                        return unsupported("variable in constant expression");
                    }
                    return Ok(
                        json!({"op":"read","slot":slot,"next":context.phase == Phase::Initial}),
                    );
                }
                if self.shareable(name, context) {
                    return self.shared_call(name, &[], context, &child);
                }
                let (body, expanded) = self.call(name, &[], context)?;
                self.expression(&body, &expanded)
            }
            E::Let { definitions, body }
                if definitions.iter().all(|definition| {
                    !definition.is_recursive
                        && matches!(&definition.head, DefinitionKind::Operator { params } if params.is_empty())
                }) =>
            {
                let mut current = child.clone();
                let mut bindings = vec![];
                for definition in definitions {
                    if current.bound >= 1000 {
                        return unsupported("more than 1000 bound variables");
                    }
                    let slot = current.bound;
                    let value = self.expression(&definition.body, &current)?;
                    current = current.nested()?;
                    current
                        .locals
                        .insert(definition.name.clone(), json!({"op":"bound","slot":slot}));
                    current.bound = slot + 1;
                    bindings.push((slot, value));
                }
                let mut result = self.expression(body, &current)?;
                for (slot, value) in bindings.into_iter().rev() {
                    result = json!({"op":"let","slot":slot,"value":value,"body":result});
                }
                Ok(result)
            }
            E::Let { definitions, body } => {
                let expanded = self.let_context(definitions, context)?;
                self.expression(body, &expanded)
            }
            E::Label { body, .. } => self.expression(body, &child),
            E::Prime { expression } if context.phase == Phase::Next => {
                let mut inner = self.expression(expression, &child)?;
                if inner["op"] == "read" && inner["next"] == false {
                    inner["next"] = json!(true);
                    Ok(inner)
                } else {
                    prime(inner)
                }
            }
            E::Unchanged { expression } if context.phase == Phase::Next => {
                let inner = self.expression(expression, &child)?;
                Ok(json!({"op":"eq","left":inner,"right":prime(inner.clone())?}))
            }
            E::Enabled { expression } => {
                let next = Context {
                    phase: Phase::Next,
                    ..child
                };
                Ok(json!({"op":"enabled","branches":self.action(expression,&next,"ENABLED")?}))
            }
            E::ExceptAt => Ok(json!({"op":"old"})),
            E::SetEnumeration { elements } | E::Tuple { elements } => Ok(
                json!({"op":if matches!(value.node,E::Tuple{..}) {"tuple"} else {"set"},"values":self.expressions(elements,&child)?}),
            ),
            E::Record { fields } | E::RecordSet { fields } => {
                let fields = fields
                    .iter()
                    .map(|field| Ok(json!([field.name, self.expression(&field.value, &child)?])))
                    .collect::<Result<Vec<_>>>()?;
                Ok(
                    json!({"op":if matches!(value.node,E::Record{..}) {"record"} else {"record-set"},"fields":fields}),
                )
            }
            E::FunctionSet { domain, range } => Ok(
                json!({"op":"function-set","left":self.expression(domain,&child)?,"right":self.expression(range,&child)?}),
            ),
            E::FunctionApplication { function, args } => {
                let key = self.key(args, &child)?;
                Ok(json!({"op":"at","target":self.expression(function,&child)?,"key":key}))
            }
            E::FieldAccess { record, field } => {
                Ok(json!({"op":"at","target":self.expression(record,&child)?,"key":string(field)}))
            }
            E::Except { function, updates } => {
                let mut lowered = vec![];
                for update in updates {
                    let mut path = vec![];
                    for part in &update.path {
                        path.push(match part {
                            ast::ExceptPath::Field { name } => string(name),
                            ast::ExceptPath::Index { args } => self.key(args, &child)?,
                        });
                    }
                    lowered
                        .push(json!({"path":path,"value":self.expression(&update.value,&child)?}));
                }
                Ok(
                    json!({"op":"update","target":self.expression(function,&child)?,"updates":lowered}),
                )
            }
            E::Forall { bounds, body }
            | E::Exists { bounds, body }
            | E::FunctionConstruction { bounds, body } => {
                let (bindings, bound) = self.bind(bounds, &child)?;
                let op = match value.node {
                    E::Forall { .. } => "forall",
                    E::Exists { .. } => "exists",
                    _ => "function",
                };
                Ok(json!({"op":op,"bounds":bindings,"body":self.expression(body,&bound)?}))
            }
            E::SetMap { bounds, expression } => {
                let (bindings, bound) = self.bind(bounds, &child)?;
                Ok(
                    json!({"op":"map-set","bounds":bindings,"body":self.expression(expression,&bound)?}),
                )
            }
            E::SetFilter { bound, predicate } => {
                let (bindings, bound) = self.bind(std::slice::from_ref(bound), &child)?;
                Ok(
                    json!({"op":"filter","bounds":bindings,"body":self.expression(predicate,&bound)?}),
                )
            }
            E::If {
                condition,
                consequent,
                alternative,
            } => Ok(
                json!({"op":"if","condition":self.expression(condition,&child)?,"consequent":self.expression(consequent,&child)?,"alternative":self.expression(alternative,&child)?}),
            ),
            E::Case {
                arms,
                other: Some(other),
            } => {
                let mut result = self.expression(other, &child)?;
                for arm in arms.iter().rev() {
                    result = json!({"op":"if","condition":self.expression(&arm.condition,&child)?,"consequent":self.expression(&arm.body,&child)?,"alternative":result});
                }
                Ok(result)
            }
            E::Case { arms, other: None } if !arms.is_empty() => {
                let (last, rest) = arms.split_last().ok_or_else(|| CompileError {
                    message: "empty CASE".into(),
                })?;
                let mut result = self.expression(&last.body, &child)?;
                for arm in rest.iter().rev() {
                    result = json!({"op":"if","condition":self.expression(&arm.condition,&child)?,"consequent":self.expression(&arm.body,&child)?,"alternative":result});
                }
                Ok(result)
            }
            E::And { operands, .. } | E::Or { operands, .. } => {
                let and = matches!(value.node, E::And { .. });
                let mut result = boolean(and);
                for operand in operands {
                    result = json!({"op":if and {"and"} else {"or"},"left":result,"right":self.expression(operand,&child)?});
                }
                Ok(result)
            }
            E::BoxAction { action, subscript } | E::AngleAction { action, subscript } => {
                let boxed = matches!(value.node, E::BoxAction { .. });
                let subscript = self.expression(subscript, &child)?;
                let unchanged =
                    json!({"op":"eq","left":subscript,"right":prime(subscript.clone())?});
                Ok(
                    json!({"op":if boxed {"or"} else {"and"},"left":self.expression(action,&child)?,"right":if boxed {unchanged} else {json!({"op":"not","argument":unchanged})}}),
                )
            }
            E::Apply { operator, args } => self.apply(operator, args, context, &child),
            _ => unsupported(format!(
                "expression at {}:{}",
                value.location.line, value.location.column
            )),
        }
    }
    fn fold(
        &mut self,
        sequence: bool,
        args: &[Expression],
        context: &Context,
        child: &Context,
    ) -> Result<Json> {
        let E::Identifier { name } = &args[0].node else {
            return unsupported("fold operator argument");
        };
        if context.locals.contains_key(name) {
            return unsupported(format!("fold operator {name}"));
        }
        let entry = self.scopes[context.scope]
            .get(name)
            .cloned()
            .ok_or_else(|| CompileError {
                message: format!("operator {name}"),
            })?;
        if entry.definition.is_recursive || context.calls.iter().any(|item| item == name) {
            return unsupported(format!("operator {name}"));
        }
        let DefinitionKind::Operator { params } = &entry.definition.head else {
            return unsupported(format!("fold operator {name}"));
        };
        if params.len() != 2 || params.iter().any(|param| param.arity != 0) {
            return unsupported(format!("fold operator {name}"));
        }
        let accumulator = context.bound;
        if accumulator + 2 > 1000 {
            return unsupported("more than 1000 bound variables");
        }
        let initial = self.expression(&args[1], child)?;
        let collection = self.expression(&args[2], child)?;
        let mut locals = entry.locals;
        locals.insert(
            params[0].name.clone(),
            json!({"op":"bound","slot":accumulator}),
        );
        locals.insert(
            params[1].name.clone(),
            json!({"op":"bound","slot":accumulator + 1}),
        );
        let mut inner = child.nested()?;
        inner.locals = locals;
        inner.scope = entry.scope;
        inner.calls.push(name.clone());
        inner.bound = accumulator + 2;
        let body = self.expression(&entry.definition.body, &inner)?;
        Ok(json!({
            "op": "fold",
            "accumulator": accumulator,
            "initial": initial,
            "collection": collection,
            "order": if sequence { "sequence-left" } else { "set-members" },
            "body": body,
        }))
    }
    fn key(&mut self, args: &[Expression], context: &Context) -> Result<Json> {
        if args.len() == 1 {
            self.expression(&args[0], context)
        } else {
            Ok(json!({"op":"tuple","values":self.expressions(args,context)?}))
        }
    }
    fn apply(
        &mut self,
        operator: &str,
        args: &[Expression],
        context: &Context,
        child: &Context,
    ) -> Result<Json> {
        if ["ApaFoldSet", "ApaFoldSeqLeft"].contains(&operator)
            && args.len() == 3
            && !self.has(context, operator)
        {
            return self.fold(operator == "ApaFoldSeqLeft", args, context, child);
        }
        if self.has(context, operator) {
            if self.shareable(operator, context) {
                return self.shared_call(operator, args, context, child);
            }
            let (body, expanded) = self.call(operator, args, context)?;
            return self.expression(&body, &expanded);
        }
        if args.len() == 1 && ["-.", "-"].contains(&operator) {
            if !self.extends("Integers") {
                return unsupported("unary minus requires Integers");
            }
            if matches!(args[0].node, E::Number { value: 2147483648 }) {
                return integer(i32::MIN as i64);
            }
        }
        if args.len() == 1 && ["~", "\\lnot", "-.", "-"].contains(&operator) {
            return Ok(
                json!({"op":if ["~","\\lnot"].contains(&operator) {"not"} else {"neg"},"argument":self.expression(&args[0],child)?}),
            );
        }
        let unary = match operator {
            "DOMAIN" => Some("domain"),
            "Cardinality" => Some("cardinality"),
            "Len" => Some("len"),
            "Head" => Some("head"),
            "Tail" => Some("tail"),
            "SUBSET" => Some("powerset"),
            "UNION" => Some("union"),
            _ => None,
        };
        if let Some(op) = unary.filter(|_| args.len() == 1) {
            if ["len", "head", "tail"].contains(&op) && !self.extends("Sequences") {
                return unsupported("sequence operator requires Sequences");
            }
            if op == "cardinality" && !self.extends("FiniteSets") {
                return unsupported("Cardinality requires FiniteSets");
            }
            return Ok(json!({"op":op,"argument":self.expression(&args[0],child)?}));
        }
        if args.len() == 2 && ["\\in", "\\notin"].contains(&operator) {
            let left = self.expression(&args[0], child)?;
            let membership = self.membership(left, &args[1], child)?;
            return Ok(if operator == "\\notin" {
                json!({"op":"not","argument":membership})
            } else {
                membership
            });
        }
        if operator == ".." && args.len() == 2 {
            let value = Expression {
                node: E::Apply {
                    operator: operator.into(),
                    args: args.to_vec(),
                },
                location: args[0].location,
            };
            return Ok(
                json!({"op":"map-set","bounds":[{"slot":context.bound,"domain":self.domain(&value,child,true)?}],"body":{"op":"bound","slot":context.bound}}),
            );
        }
        if operator == "Append" && args.len() == 2 && self.extends("Sequences") {
            return Ok(
                json!({"op":"append","left":self.expression(&args[0],child)?,"right":self.expression(&args[1],child)?}),
            );
        }
        if operator == "SubSeq" && args.len() == 3 && self.extends("Sequences") {
            return Ok(
                json!({"op":"subseq","sequence":self.expression(&args[0],child)?,"from":self.expression(&args[1],child)?,"to":self.expression(&args[2],child)?}),
            );
        }
        let op = match operator {
            "+" => "add",
            "-" => "sub",
            "*" => "mul",
            "=" => "eq",
            "#" | "/=" => "ne",
            "<" => "lt",
            "<=" | "\\leq" => "le",
            ">" => "gt",
            ">=" | "\\geq" => "ge",
            "=>" => "implies",
            "\\cup" | "\\union" => "cup",
            "\\cap" | "\\intersect" => "cap",
            "\\" => "difference",
            "\\subseteq" => "subset-eq",
            "\\o" => "concat",
            _ => return unsupported(format!("operator {operator}")),
        };
        if args.len() != 2 {
            return unsupported(format!("operator arity {operator}"));
        }
        if ["add", "sub", "mul", "lt", "le", "gt", "ge"].contains(&op) && !self.arithmetic() {
            return unsupported("arithmetic requires Naturals");
        }
        if op == "concat" && !self.extends("Sequences") {
            return unsupported("concatenation requires Sequences");
        }
        Ok(
            json!({"op":op,"left":self.expression(&args[0],child)?,"right":self.expression(&args[1],child)?}),
        )
    }
    fn membership_set(&mut self, value: &Expression, context: &Context) -> Result<Json> {
        let child = context.nested()?;
        match &value.node {
            E::Identifier { name }
                if self.has(context, name) && !context.locals.contains_key(name) =>
            {
                let (body, expanded) = self.call(name, &[], context)?;
                self.membership_set(&body, &expanded)
            }
            E::Apply { operator, args }
                if operator == "Seq" && args.len() == 1 && self.extends("Sequences") =>
            {
                Ok(json!({"op":"sequence-set","argument":self.membership_set(&args[0],&child)?}))
            }
            E::FunctionSet { domain, range } => Ok(
                json!({"op":"function-set","left":self.expression(domain,&child)?,"right":self.membership_set(range,&child)?}),
            ),
            E::RecordSet { fields } => {
                let fields = fields
                    .iter()
                    .map(|field| {
                        Ok(json!([
                            field.name,
                            self.membership_set(&field.value, &child)?
                        ]))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(json!({"op":"record-set","fields":fields}))
            }
            _ => self.expression(value, context),
        }
    }
    fn membership(&mut self, left: Json, value: &Expression, context: &Context) -> Result<Json> {
        match &value.node {
            E::Identifier { name }
                if self.has(context, name) && !context.locals.contains_key(name) =>
            {
                let (body, expanded) = self.call(name, &[], context)?;
                self.membership(left, &body, &expanded)
            }
            E::Apply { operator, args }
                if operator == "Seq" && args.len() == 1 && self.extends("Sequences") =>
            {
                Ok(
                    json!({"op":"sequence-in","left":left,"right":self.membership_set(&args[0],&context.nested()?)?}),
                )
            }
            E::RecordSet { .. } | E::FunctionSet { .. } => Ok(
                json!({"op":"function-in","left":left,"right":self.membership_set(value,&context.nested()?)?}),
            ),
            E::Apply { operator, args } if operator == "SUBSET" && args.len() == 1 => {
                Ok(json!({"op":"subset-eq","left":left,"right":self.expression(&args[0],context)?}))
            }
            _ => Ok(json!({"op":"in","value":left,"domain":self.domain(value,context,false)?})),
        }
    }
    fn target(&self, value: &Expression, context: &Context) -> Option<usize> {
        match &value.node {
            E::Identifier { name }
                if context.phase == Phase::Initial && !context.locals.contains_key(name) =>
            {
                self.slots.get(name).copied()
            }
            E::Prime { expression } if context.phase == Phase::Next => match &expression.node {
                E::Identifier { name } if !context.locals.contains_key(name) => {
                    self.slots.get(name).copied()
                }
                _ => None,
            },
            _ => None,
        }
    }
    fn action(&mut self, value: &Expression, context: &Context, name: &str) -> Result<Vec<Json>> {
        let child = context.nested()?;
        match &value.node {
            E::Identifier { name } if self.has(context, name) => {
                let (body, expanded) = self.call(name, &[], context)?;
                self.action(&body, &expanded, name)
            }
            E::Apply { operator, args } if self.has(context, operator) => {
                let (body, expanded) = self.call(operator, args, context)?;
                self.action(&body, &expanded, operator)
            }
            E::Let { definitions, body }
                if definitions.iter().all(|definition| {
                    !definition.is_recursive
                        && matches!(&definition.head, DefinitionKind::Operator { params } if params.is_empty())
                }) =>
            {
                let mut current = child.clone();
                let mut prefix = vec![];
                for definition in definitions {
                    if current.bound >= 1000 {
                        return unsupported("more than 1000 bound variables");
                    }
                    let slot = current.bound;
                    let value = self.expression(&definition.body, &current)?;
                    let stable = level(&value) <= 1;
                    prefix.push(json!({"kind":"let","slot":slot,"value":value,"stable":stable}));
                    current = current.nested()?;
                    current
                        .locals
                        .insert(definition.name.clone(), json!({"op":"bound","slot":slot}));
                    current.bound = slot + 1;
                }
                let mut branches = self.action(body, &current, name)?;
                for branch in &mut branches {
                    let mut joined = prefix.clone();
                    joined.extend(steps(branch)?.clone());
                    branch["steps"] = json!(joined);
                }
                limited(branches)
            }
            E::Let { definitions, body } => {
                let expanded = self.let_context(definitions, context)?;
                self.action(body, &expanded, name)
            }
            E::Label { body, .. } => self.action(body, &child, name),
            E::Exists { bounds, body } => {
                let (bindings, bound) = self.bind(bounds, &child)?;
                let mut prefix = vec![];
                for mut binding in bindings {
                    binding["kind"] = json!("bind");
                    prefix.push(binding);
                }
                let mut branches = self.action(body, &bound, name)?;
                for branch in &mut branches {
                    let mut joined = prefix.clone();
                    joined.extend(steps(branch)?.clone());
                    branch["steps"] = json!(joined);
                }
                limited(branches)
            }
            E::Or { operands, .. } => {
                let mut result = vec![];
                for operand in operands {
                    result.extend(self.action(operand, &child, name)?);
                    result = limited(result)?;
                }
                Ok(result)
            }
            E::And { operands, .. } => {
                let mut result = vec![json!({"name":name,"steps":[]})];
                for operand in operands {
                    let mut next = vec![];
                    for branch in &result {
                        let bound = context.bound
                            + steps(branch)?
                                .iter()
                                .filter(|step| step["kind"] == "bind" || step["kind"] == "let")
                                .count();
                        for mut addition in self.action(
                            operand,
                            &Context {
                                bound,
                                ..child.clone()
                            },
                            name,
                        )? {
                            let mut joined = steps(branch)?.clone();
                            joined.extend(steps(&addition)?.clone());
                            addition["steps"] = json!(joined);
                            next.push(addition);
                            if next.len() > 4096 {
                                return unsupported("action expansion exceeds 4096 branches");
                            }
                        }
                    }
                    result = limited(next)?;
                }
                Ok(result)
            }
            E::If {
                condition,
                consequent,
                alternative,
            } => {
                let condition = self.expression(condition, &child)?;
                let mut result = vec![];
                for (body, guard) in [
                    (consequent, condition.clone()),
                    (alternative, json!({"op":"not","argument":condition})),
                ] {
                    for mut branch in self.action(body, &child, name)? {
                        let mut joined = vec![json!({"kind":"guard","expression":guard})];
                        joined.extend(steps(&branch)?.clone());
                        branch["steps"] = json!(joined);
                        result.push(branch);
                    }
                }
                limited(result)
            }
            E::Case { arms, other } if !arms.is_empty() => {
                let mut result = vec![];
                let mut previous: Vec<Json> = vec![];
                for arm in arms {
                    let condition = self.expression(&arm.condition, &child)?;
                    let mut guards: Vec<Json> = previous
                        .iter()
                        .map(|earlier| json!({"op":"not","argument":earlier}))
                        .collect();
                    guards.push(condition.clone());
                    for mut branch in self.action(&arm.body, &child, name)? {
                        let mut joined: Vec<Json> = guards
                            .iter()
                            .map(|guard| json!({"kind":"guard","expression":guard}))
                            .collect();
                        joined.extend(steps(&branch)?.clone());
                        branch["steps"] = json!(joined);
                        result.push(branch);
                    }
                    previous.push(condition);
                    result = limited(result)?;
                }
                if let Some(other) = other {
                    let guards: Vec<Json> = previous
                        .iter()
                        .map(|earlier| json!({"kind":"guard","expression":{"op":"not","argument":earlier}}))
                        .collect();
                    for mut branch in self.action(other, &child, name)? {
                        let mut joined = guards.clone();
                        joined.extend(steps(&branch)?.clone());
                        branch["steps"] = json!(joined);
                        result.push(branch);
                    }
                }
                limited(result)
            }
            E::Unchanged { expression } if context.phase == Phase::Next => Ok(vec![
                json!({"name":name,"steps":unchanged(self.expression(expression,&child)?)?}),
            ]),
            E::Apply { operator, args } if args.len() == 2 => {
                if let Some(slot) = self.target(&args[0], context) {
                    if operator == "=" {
                        return Ok(vec![
                            json!({"name":name,"steps":[{"kind":"assign","slot":slot,"value":self.expression(&args[1],&child)?}]}),
                        ]);
                    }
                    if operator == "\\in" {
                        return Ok(vec![
                            json!({"name":name,"steps":[{"kind":"choose","slot":slot,"domain":self.domain(&args[1],&child,true)?}]}),
                        ]);
                    }
                }
                if operator == "="
                    && let Some(slot) = self.target(&args[1], context)
                {
                    return Ok(vec![
                        json!({"name":name,"steps":[{"kind":"assign","slot":slot,"value":self.expression(&args[0],&child)?}]}),
                    ]);
                }
                Ok(vec![
                    json!({"name":name,"steps":[{"kind":"guard","expression":self.expression(value,context)?}]}),
                ])
            }
            _ => Ok(vec![
                json!({"name":name,"steps":[{"kind":"guard","expression":self.expression(value,context)?}]}),
            ]),
        }
    }
    fn boxed_action(&mut self, value: &Expression, context: &Context) -> Result<bool> {
        match &value.node {
            E::Identifier { name }
                if self.has(context, name) && !context.locals.contains_key(name) =>
            {
                let (body, expanded) = self.call(name, &[], context)?;
                self.boxed_action(&body, &expanded)
            }
            E::Apply { operator, args } if self.has(context, operator) => {
                let (body, expanded) = self.call(operator, args, context)?;
                self.boxed_action(&body, &expanded)
            }
            E::BoxAction { .. } => Ok(true),
            _ => Ok(false),
        }
    }
    fn temporal(&mut self, value: &Expression, context: &Context) -> Result<Json> {
        let child = context.nested()?;
        match &value.node {
            E::Identifier { name } if self.has(context, name) => {
                let (body, expanded) = self.call(name, &[], context)?;
                self.temporal(&body, &expanded)
            }
            E::Apply { operator, args } if self.has(context, operator) => {
                let (body, expanded) = self.call(operator, args, context)?;
                self.temporal(&body, &expanded)
            }
            E::Let { definitions, body } => {
                let expanded = self.let_context(definitions, context)?;
                self.temporal(body, &expanded)
            }
            E::Always { expression } | E::Eventually { expression } => {
                let formula = self.temporal(expression, &child)?;
                let always = matches!(value.node, E::Always { .. });
                if always
                    && formula["kind"] == "atom"
                    && level(&formula["expression"]) > 1
                    && !self.boxed_action(expression, &child)?
                {
                    return unsupported("[] action must have the form [A]_v");
                }
                Ok(json!({"kind":if always {"always"} else {"eventually"},"formula":formula}))
            }
            E::And { operands, .. } | E::Or { operands, .. } => {
                let and = matches!(value.node, E::And { .. });
                let mut result = atom(boolean(and));
                for operand in operands {
                    result = combine(
                        if and { "and" } else { "or" },
                        result,
                        self.temporal(operand, &child)?,
                    );
                }
                Ok(result)
            }
            E::Apply { operator, args }
                if args.len() == 1 && ["~", "\\lnot"].contains(&operator.as_str()) =>
            {
                Ok(negate(self.temporal(&args[0], &child)?))
            }
            E::Apply { operator, args }
                if args.len() == 2 && ["=>", "~>"].contains(&operator.as_str()) =>
            {
                let right = self.temporal(&args[1], &child)?;
                let result = combine(
                    "or",
                    negate(self.temporal(&args[0], &child)?),
                    if operator == "~>" {
                        json!({"kind":"eventually","formula":right})
                    } else {
                        right
                    },
                );
                Ok(if operator == "~>" {
                    json!({"kind":"always","formula":result})
                } else {
                    result
                })
            }
            E::WeakFairness { subscript, action } | E::StrongFairness { subscript, action } => {
                let subscript = self.expression(subscript, &child)?;
                let enabled = atom(
                    json!({"op":"enabled","branches":self.action(action,&child,"fairness")?,"subscript":subscript}),
                );
                let disabled = json!({"kind":"not","formula":enabled});
                let taken = atom(
                    json!({"op":"and","left":self.expression(action,&child)?,"right":{"op":"ne","left":subscript,"right":prime(subscript.clone())?}}),
                );
                Ok(if matches!(value.node, E::WeakFairness { .. }) {
                    json!({"kind":"always","formula":{"kind":"eventually","formula":{"kind":"or","left":disabled,"right":taken}}})
                } else {
                    json!({"kind":"or","left":{"kind":"eventually","formula":{"kind":"always","formula":disabled}},"right":{"kind":"always","formula":{"kind":"eventually","formula":taken}}})
                })
            }
            _ => Ok(atom(self.expression(value, context)?)),
        }
    }
    fn parts(
        &mut self,
        value: &Expression,
        context: &Context,
        initial: &mut Option<Vec<Json>>,
        next: &mut Option<Vec<Json>>,
        fairness: &mut Vec<Json>,
    ) -> Result<()> {
        match &value.node {
            E::Identifier { name } if self.has(context, name) => {
                let (body, expanded) = self.call(name, &[], context)?;
                self.parts(&body, &expanded, initial, next, fairness)
            }
            E::And { operands, .. } => {
                for operand in operands {
                    self.parts(operand, &context.nested()?, initial, next, fairness)?;
                }
                Ok(())
            }
            E::Always { expression } if matches!(expression.node, E::BoxAction { .. }) => {
                if next.is_some() {
                    return unsupported("multiple NEXT conjuncts");
                }
                if let E::BoxAction { action, subscript } = &expression.node {
                    let subscript = self.expression(subscript, context)?;
                    let mut reads = HashSet::new();
                    read_slots(&subscript, &mut reads)?;
                    if reads.len() != self.slots.len() {
                        return unsupported(
                            "SPECIFICATION subscript must cover every state variable",
                        );
                    }
                    *next = Some(self.action(action, context, "Next")?);
                }
                Ok(())
            }
            E::WeakFairness { .. } | E::StrongFairness { .. } => {
                fairness.push(self.temporal(value, context)?);
                Ok(())
            }
            _ => {
                let additions = self.action(
                    value,
                    &Context {
                        phase: Phase::Initial,
                        ..context.clone()
                    },
                    "Init",
                )?;
                *initial = Some(if let Some(previous) = initial.take() {
                    let mut result = vec![];
                    for branch in &previous {
                        for addition in &additions {
                            let mut joined = steps(branch)?.clone();
                            joined.extend(steps(addition)?.clone());
                            result.push(json!({"name":addition["name"],"steps":joined}));
                            if result.len() > 4096 {
                                return unsupported("action expansion exceeds 4096 branches");
                            }
                        }
                    }
                    limited(result)?
                } else {
                    additions
                });
                Ok(())
            }
        }
    }
    fn operator(&mut self, name: &str, phase: Phase) -> Result<(Expression, Context)> {
        self.call(name, &[], &self.root(phase))
    }
    fn program(&mut self) -> Result<Json> {
        let mut initial = None;
        let mut next = None;
        let mut fairness = vec![];
        if let Some(name) = self.config.specification.clone() {
            let (body, context) = self.operator(&name, Phase::Next)?;
            self.parts(&body, &context, &mut initial, &mut next, &mut fairness)?;
        } else {
            for (phase, name) in [
                (Phase::Initial, self.config.init.clone()),
                (Phase::Next, self.config.next.clone()),
            ] {
                let name = name.ok_or_else(|| CompileError {
                    message: "INIT / NEXT".into(),
                })?;
                let (body, context) = self.operator(&name, phase)?;
                let branches = self.action(&body, &context, &name)?;
                if phase == Phase::Initial {
                    initial = Some(branches);
                } else {
                    next = Some(branches);
                }
            }
        }
        let initial = initial.ok_or_else(|| CompileError {
            message: "initial predicate required".into(),
        })?;
        let next = next.ok_or_else(|| CompileError {
            message: "NEXT required".into(),
        })?;
        if !next.iter().any(|branch| {
            steps(branch).is_ok_and(|steps| {
                steps
                    .iter()
                    .any(|step| step["kind"] != "guard" || level(&step["expression"]) > 0)
            })
        }) {
            return unsupported("NEXT cannot be constant");
        }
        let mut invariants = vec![];
        for name in self.config.invariants.clone() {
            let (body, context) = self.operator(&name, Phase::State)?;
            invariants.push(json!({"name":name,"expression":self.expression(&body,&context)?}));
        }
        let mut initial_properties = vec![];
        let mut action_properties = vec![];
        let mut properties = vec![];
        for name in self.config.properties.clone() {
            let (body, context) = self.operator(&name, Phase::Next)?;
            let formula = self.temporal(&body, &context)?;
            validate_temporal(
                &formula,
                &[],
                formula["kind"] == "always" && formula["formula"]["kind"] == "atom",
            )?;
            if formula["kind"] == "atom" {
                if level(&formula["expression"]) > 1 {
                    return unsupported("initial PROPERTY must be a state predicate");
                }
                initial_properties.push(json!({"name":name,"expression":formula["expression"]}));
            } else if formula["kind"] == "always" && formula["formula"]["kind"] == "atom" {
                let expression = formula["formula"]["expression"].clone();
                let target = if level(&expression) > 1 {
                    &mut action_properties
                } else {
                    &mut invariants
                };
                target.push(json!({"name":name,"expression":expression}));
            } else {
                properties.push(json!({"name":name,"formula":formula}));
            }
        }
        let mut assumptions = vec![];
        for (index, assumption) in self.module.assumptions.iter().enumerate() {
            assumptions.push(json!({"name":assumption.name.clone().unwrap_or_else(||format!("ASSUME {}",index+1)),"expression":self.expression(&assumption.expression,&self.root(Phase::Constant))?}));
        }
        let mut result = json!({"format":"modelt.safety-program","version":1,"name":self.module.name,"variables":self.module.variables.iter().map(|value|&value.name).collect::<Vec<_>>(),"initial":initial,"next":next,"invariants":invariants,"assumptions":assumptions,"checkDeadlock":self.config.check_deadlock});
        if !initial_properties.is_empty() {
            result["initialProperties"] = json!(initial_properties);
        }
        if !action_properties.is_empty() {
            result["actionProperties"] = json!(action_properties);
        }
        if !properties.is_empty() {
            result["properties"] = json!(properties);
            result["fairness"] = json!(fairness);
        }
        if !self.operators.is_empty() {
            result["operators"] = json!(self.operators);
        }
        Ok(result)
    }
}
fn read_slots(value: &Json, slots: &mut HashSet<u64>) -> Result<()> {
    if value["op"] == "tuple" {
        for inner in value["values"].as_array().into_iter().flatten() {
            read_slots(inner, slots)?;
        }
        return Ok(());
    }
    if value["op"] != "read" || value["next"] != false {
        return unsupported("SPECIFICATION subscript must list state variables");
    }
    if let Some(slot) = value["slot"].as_u64() {
        slots.insert(slot);
    }
    Ok(())
}
fn validate_temporal(formula: &Json, path: &[String], direct: bool) -> Result<()> {
    match formula["kind"].as_str() {
        Some("atom") => {
            let tail = path
                .iter()
                .rev()
                .take(2)
                .map(String::as_str)
                .collect::<Vec<_>>();
            if level(&formula["expression"]) > 1
                && !(direct && path.len() == 1)
                && tail != ["eventually", "always"]
                && tail != ["always", "eventually"]
            {
                return unsupported("temporal actions require []<>A or <>[]A");
            }
            Ok(())
        }
        Some("and" | "or") => {
            validate_temporal(&formula["left"], path, direct)?;
            validate_temporal(&formula["right"], path, direct)
        }
        Some(kind) => {
            let mut path = path.to_vec();
            if kind != "not" {
                path.push(kind.into());
            }
            validate_temporal(&formula["formula"], &path, direct)
        }
        None => unsupported("invalid temporal formula"),
    }
}
