use crate::safety_expressions::{EvalContext, validate_branches, visit_branch};
#[cfg(not(target_arch = "wasm32"))]
#[path = "safety_parallel.rs"]
mod parallel;
use crate::ir::{
    Branch, SafetyBounds, SafetyProgram, SafetyResult, SafetyTraceStep, SafetyViolation, Scalar,
};
use crate::safety_values::Arena;
use crate::temporal::GraphEdge;
use crate::{CoreError, Value};
use std::collections::{HashMap, HashSet};

struct Node {
    parent: Option<usize>,
    action: Option<usize>,
    depth: u32,
}

#[derive(Clone, Copy)]
struct BranchCursor<'a> {
    definition: &'a Branch,
    index: usize,
}

struct StateStore {
    width: usize,
    values: Vec<Scalar>,
    heads: HashMap<u64, usize>,
    links: Vec<Option<usize>>,
}

impl StateStore {
    fn new(width: usize) -> Self {
        Self {
            width,
            values: vec![],
            heads: HashMap::new(),
            links: vec![],
        }
    }
    fn state(&self, index: usize) -> &[Scalar] {
        &self.values[index * self.width..(index + 1) * self.width]
    }
    fn find(&self, hash: u64, values: &[Option<Scalar>]) -> Option<usize> {
        if values.len() != self.width {
            return None;
        }
        let mut candidate = self.heads.get(&hash).copied();
        while let Some(index) = candidate {
            if self
                .state(index)
                .iter()
                .zip(values)
                .all(|(stored, actual)| Some(*stored) == *actual)
            {
                return Some(index);
            }
            candidate = self.links[index];
        }
        None
    }
    fn push(&mut self, hash: u64, values: &[Option<Scalar>]) -> Result<usize, CoreError> {
        if values.len() != self.width || values.iter().any(Option::is_none) {
            return Err(CoreError::protocol("Incomplete safety state"));
        }
        let index = self.links.len();
        self.values.extend(values.iter().flatten().copied());
        self.links.push(self.heads.insert(hash, index));
        Ok(index)
    }
}

fn get_hash(values: &[Option<Scalar>]) -> Result<u64, CoreError> {
    values
        .iter()
        .try_fold(0xcbf2_9ce4_8422_2325_u64, |hash, value| {
            let bits = match value.ok_or_else(|| CoreError::protocol("Incomplete safety state"))? {
                Scalar::Int(value) => u64::from(value as u32) << 1,
                Scalar::Bool(value) => (u64::from(value) << 1) | 1,
                Scalar::Ref(value) => u64::from(value) ^ 0x9e37_79b9_7f4a_7c15,
            };
            Ok(hash.wrapping_mul(0x0000_0100_0000_01b3) ^ bits)
        })
}

struct Search<'a> {
    program: &'a SafetyProgram,
    bounds: SafetyBounds,
    nodes: Vec<Node>,
    store: StateStore,
    result: SafetyResult,
    graph: Option<Vec<Vec<GraphEdge>>>,
    roots: Vec<usize>,
}

impl SafetyProgram {
    pub fn check(&self, bounds: SafetyBounds) -> Result<SafetyResult, CoreError> {
        self.check_with_workers(bounds, 1)
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn check_parallel(
        &self,
        bounds: SafetyBounds,
        workers: usize,
    ) -> Result<SafetyResult, CoreError> {
        if workers == 0 || workers > 256 {
            return Err(CoreError::protocol(
                "Safety workers must be between 1 and 256",
            ));
        }
        self.check_with_workers(bounds, workers)
    }
    fn check_with_workers(
        &self,
        bounds: SafetyBounds,
        workers: usize,
    ) -> Result<SafetyResult, CoreError> {
        if self.format != "modelt.safety-program"
            || self.version != 1
            || self.name.is_empty()
            || self.variables.is_empty()
            || self.variables.len() > 256
            || self.variables.iter().any(String::is_empty)
            || self.variables.iter().collect::<HashSet<_>>().len() != self.variables.len()
            || bounds.max_states == 0
            || bounds.max_work == 0
        {
            return Err(CoreError::protocol("Invalid safety program or bounds"));
        }
        let mut arena = Arena::new(bounds.max_work);
        arena.operators = &self.operators;
        let mut search = Search {
            program: self,
            bounds,
            nodes: vec![],
            graph: (!self.properties.is_empty()).then(Vec::new),
            roots: vec![],
            store: StateStore::new(self.variables.len()),
            result: SafetyResult {
                status: "complete".into(),
                reason: None,
                states: 0,
                generated: 0,
                depth: 0,
                work: 0,
                violation: None,
            },
        };
        let outcome = search.run(&mut arena, workers);
        if let Err(error) = outcome {
            if error.code != "budget" && error.code != "temporal-budget" {
                return Err(error);
            }
            search.result.status = "bounded".into();
            search.result.reason = Some(
                if error.code == "temporal-budget" {
                    "temporal"
                } else {
                    "work"
                }
                .into(),
            );
        }
        search.result.work = arena.work;
        Ok(search.result)
    }
}

impl<'program> Search<'program> {
    fn run(&mut self, arena: &mut Arena<'program>, workers: usize) -> Result<(), CoreError> {
        let program = self.program;
        validate_branches(&program.initial, program.variables.len(), 0, 0)?;
        validate_branches(&program.next, program.variables.len(), 1, 0)?;
        for definition in &program.operators {
            if definition.phase > 3 || definition.arity > 256 {
                return Err(CoreError::protocol("Invalid operator definition"));
            }
            definition.body.validate_with_locals(
                program.variables.len(),
                definition.phase,
                definition.arity,
            )?;
            arena.register_memberships(&definition.body);
        }
        crate::safety_expressions::register_branch_memberships(
            &program.initial,
            &mut |expression| arena.register_memberships(expression),
        );
        crate::safety_expressions::register_branch_memberships(&program.next, &mut |expression| {
            arena.register_memberships(expression)
        });
        for predicate in program.invariants.iter().chain(&program.initial_properties) {
            predicate
                .expression
                .validate(program.variables.len(), 2, 0)?;
            arena.register_memberships(&predicate.expression);
            if predicate.expression.is_constant()
                && !predicate
                    .expression
                    .eval(EvalContext::new(&[], &[]), arena)?
                    .boolean()?
            {
                return Err(CoreError::protocol(
                    "Constant invariant or property is false",
                ));
            }
        }
        for predicate in &program.action_properties {
            predicate
                .expression
                .validate(program.variables.len(), 1, 0)?;
            arena.register_memberships(&predicate.expression);
        }
        for property in &program.properties {
            property.formula.validate(program.variables.len(), 0)?;
        }
        for fairness in &program.fairness {
            fairness.validate(program.variables.len(), 0)?;
        }
        for predicate in &program.assumptions {
            predicate
                .expression
                .validate(program.variables.len(), 3, 0)?;
            arena.register_memberships(&predicate.expression);
            if !predicate
                .expression
                .eval(EvalContext::new(&[], &[]), arena)?
                .boolean()?
            {
                self.result.status = "assumption-violation".into();
                self.result.violation = Some(SafetyViolation {
                    name: predicate.name.clone(),
                    trace: vec![],
                    loop_start: None,
                    loop_action: None,
                });
                return Ok(());
            }
        }
        let mut next = vec![None; program.variables.len()];
        for (index, branch) in program.initial.iter().enumerate() {
            self.walk(
                arena,
                BranchCursor {
                    definition: branch,
                    index,
                },
                0,
                EvalContext::new(&[], &[]),
                &mut next,
                None,
                &mut false,
            )?;
            if self.stopped() {
                return Ok(());
            }
        }
        if self.nodes.is_empty() {
            self.result.status = "empty".into();
            return Ok(());
        }
        let mut index = 0;
        let mut current = vec![Scalar::Int(0); program.variables.len()];
        #[cfg(not(target_arch = "wasm32"))]
        let mut pool = None;
        #[cfg(not(target_arch = "wasm32"))]
        let parallel = workers > 1 && parallel::is_expensive(program);
        #[cfg(target_arch = "wasm32")]
        let _ = workers;
        while index < self.nodes.len() {
            let end = self.nodes.len();
            #[cfg(not(target_arch = "wasm32"))]
            if parallel
                && end - index >= 16
                && program.action_properties.is_empty()
                && program.properties.is_empty()
                && program.fairness.is_empty()
            {
                if pool.is_none() {
                    pool = Some(parallel::Workers::new(workers, self.store.width)?);
                }
                self.parallel_frontier(index..end, pool.as_mut().unwrap(), arena)?;
                if self.stopped() {
                    return Ok(());
                }
                index = end;
                continue;
            }
            while index < end {
                self.expand(index, arena, &mut current, &mut next)?;
                if self.stopped() {
                    return Ok(());
                }
                index += 1;
            }
        }
        if self.result.status == "complete"
            && let Some(graph) = &mut self.graph
        {
            for (index, edges) in graph.iter_mut().enumerate() {
                edges.push(GraphEdge {
                    target: index,
                    action: None,
                });
            }
            if let Some(violation) = crate::temporal::check(
                program,
                self.bounds,
                &self.store.values,
                self.store.width,
                graph,
                &self.roots,
                arena,
            )? {
                self.result.status = "liveness-violation".into();
                self.result.violation = Some(violation);
            }
        }
        Ok(())
    }
    fn stopped(&self) -> bool {
        self.result.violation.is_some()
            || self.result.reason.as_deref() == Some("work")
            || self.result.reason.as_deref() == Some("states")
    }
    fn expand(
        &mut self,
        index: usize,
        arena: &mut Arena,
        current: &mut [Scalar],
        next: &mut [Option<Scalar>],
    ) -> Result<(), CoreError> {
        let program = self.program;
        current.copy_from_slice(self.store.state(index));
        let mut enabled = false;
        for (branch_index, branch) in program.next.iter().enumerate() {
            self.walk(
                arena,
                BranchCursor {
                    definition: branch,
                    index: branch_index,
                },
                0,
                EvalContext::new(current, &[]),
                next,
                Some(index),
                &mut enabled,
            )?;
            if self.stopped() {
                return Ok(());
            }
        }
        if !enabled && program.check_deadlock {
            self.deadlock(index, arena)?;
        }
        Ok(())
    }
    fn deadlock(&mut self, index: usize, arena: &Arena) -> Result<(), CoreError> {
        self.result.status = "deadlock".into();
        self.result.reason = None;
        self.result.violation = Some(SafetyViolation {
            name: "deadlock".into(),
            trace: self.trace(index, arena)?,
            loop_start: None,
            loop_action: None,
        });
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        arena: &mut Arena,
        branch: BranchCursor<'_>,
        position: usize,
        context: EvalContext<'_>,
        next: &mut [Option<Scalar>],
        parent: Option<usize>,
        enabled: &mut bool,
    ) -> Result<(), CoreError> {
        visit_branch(
            branch.definition,
            position,
            context,
            next,
            arena,
            &mut |values, arena| {
                *enabled = true;
                self.accept(values, parent, branch.index, arena)?;
                Ok(self.stopped())
            },
        )?;
        Ok(())
    }
    fn accept(
        &mut self,
        values: &[Option<Scalar>],
        parent: Option<usize>,
        action: usize,
        arena: &mut Arena,
    ) -> Result<(), CoreError> {
        let hash = get_hash(values)?;
        self.result.generated += 1;
        let existing = self.store.find(hash, values);
        let index = if let Some(index) = existing {
            index
        } else {
            let depth = parent.map_or(0, |index| self.nodes[index].depth + 1);
            if depth > self.bounds.max_depth {
                self.result.status = "bounded".into();
                self.result.reason = Some("depth".into());
                return Ok(());
            }
            if self.nodes.len() == self.bounds.max_states as usize {
                self.result.status = "bounded".into();
                self.result.reason = Some("states".into());
                return Ok(());
            }
            let index = self.store.push(hash, values)?;
            self.nodes.push(Node {
                parent,
                action: parent.map(|_| action),
                depth,
            });
            if let Some(graph) = &mut self.graph {
                graph.push(vec![]);
            }
            self.result.states = self.nodes.len();
            self.result.depth = self.result.depth.max(depth);
            index
        };
        if let Some(parent) = parent {
            if let Some(graph) = &mut self.graph {
                graph[parent].push(GraphEdge {
                    target: index,
                    action: Some(action),
                });
            }
        } else if !self.roots.contains(&index) {
            self.roots.push(index);
        }
        if existing.is_none() {
            let violation = check_invariants(self.program, self.store.state(index), arena)?;
            if let Some(predicate) = violation {
                self.result.status = "invariant-violation".into();
                self.result.reason = None;
                self.result.violation = Some(SafetyViolation {
                    name: self.program.invariants[predicate].name.clone(),
                    trace: self.trace(index, arena)?,
                    loop_start: None,
                    loop_action: None,
                });
                return Ok(());
            }
        }
        if let Some(parent) = parent {
            for predicate in &self.program.action_properties {
                if !predicate
                    .expression
                    .eval(EvalContext::new(self.store.state(parent), values), arena)?
                    .boolean()?
                {
                    let mut trace = self.trace(parent, arena)?;
                    trace.push(SafetyTraceStep {
                        action: Some(self.program.next[action].name.clone()),
                        values: self.portable(index, arena)?,
                    });
                    self.result.status = "property-violation".into();
                    self.result.reason = None;
                    self.result.violation = Some(SafetyViolation {
                        name: predicate.name.clone(),
                        trace,
                        loop_start: None,
                        loop_action: None,
                    });
                    return Ok(());
                }
            }
        } else {
            for predicate in &self.program.initial_properties {
                if !predicate
                    .expression
                    .eval(EvalContext::new(self.store.state(index), &[]), arena)?
                    .boolean()?
                {
                    self.result.status = "property-violation".into();
                    self.result.reason = None;
                    self.result.violation = Some(SafetyViolation {
                        name: predicate.name.clone(),
                        trace: self.trace(index, arena)?,
                        loop_start: None,
                        loop_action: None,
                    });
                    return Ok(());
                }
            }
        }
        Ok(())
    }
    fn portable(&self, index: usize, arena: &Arena) -> Result<Vec<Value>, CoreError> {
        self.store
            .state(index)
            .iter()
            .map(|value| arena.portable(*value))
            .collect()
    }
    fn trace(&self, mut index: usize, arena: &Arena) -> Result<Vec<SafetyTraceStep>, CoreError> {
        let mut trace = vec![];
        loop {
            let node = &self.nodes[index];
            trace.push(SafetyTraceStep {
                action: node
                    .action
                    .map(|index| self.program.next[index].name.clone()),
                values: self.portable(index, arena)?,
            });
            match node.parent {
                Some(parent) => index = parent,
                None => break,
            }
        }
        trace.reverse();
        Ok(trace)
    }
}

fn check_invariants(
    program: &SafetyProgram,
    values: &[Scalar],
    arena: &mut Arena,
) -> Result<Option<usize>, CoreError> {
    for (index, predicate) in program.invariants.iter().enumerate() {
        if !predicate
            .expression
            .eval(EvalContext::new(values, &[]), arena)?
            .boolean()?
        {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{Scalar, StateStore};

    #[test]
    fn colliding_hashes_keep_distinct_typed_states() {
        let mut store = StateStore::new(2);
        let first = [Some(Scalar::Int(0)), Some(Scalar::Int(1))];
        let second = [Some(Scalar::Int(0)), Some(Scalar::Int(2))];
        let third = [Some(Scalar::Bool(false)), Some(Scalar::Int(1))];
        for values in [&first, &second, &third] {
            store.push(0, values).unwrap();
        }
        assert_eq!(store.find(0, &first), Some(0));
        assert_eq!(store.find(0, &second), Some(1));
        assert_eq!(store.find(0, &third), Some(2));
        assert_eq!(
            store.find(0, &[Some(Scalar::Int(0)), Some(Scalar::Int(3))]),
            None
        );
        assert_eq!(store.find(0, &first[..1]), None);
        assert!(store.push(0, &[Some(Scalar::Int(0)), None]).is_err());
        assert_eq!(store.find(0, &third), Some(2));
    }
}
