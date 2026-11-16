use serde::Deserialize;
use serde_json::{Value, json};
use varisat::{ExtendFormula, Lit, Solver};
#[cfg(target_arch = "wasm32")]
mod wasm;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    variables: usize,
    clauses: Vec<Vec<i32>>,
    rounds: Vec<Round>,
    #[serde(rename = "maxWork")]
    max_work: Option<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Round {
    add: Vec<Vec<i32>>,
    assumptions: Vec<i32>,
}

pub fn dispatch(bytes: &[u8]) -> Value {
    match solve(bytes) {
        Ok(result) => result,
        Err(message) => json!({"kind":"error","message":message}),
    }
}
fn literals(input: &[i32], variables: usize) -> Result<Vec<Lit>, String> {
    input
        .iter()
        .map(|literal| {
            if *literal == 0 || literal.unsigned_abs() as usize > variables {
                return Err("literal outside declared variables".into());
            }
            Ok(Lit::from_dimacs(*literal as isize))
        })
        .collect()
}
fn solve(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > 1_048_576 {
        return Err("request exceeds 1 MiB".into());
    }
    let input: Input = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if input.max_work.is_some() {
        return Ok(
            json!({"kind":"unsupported","reason":"work-budget","candidate":"varisat-0.2.2"}),
        );
    }
    if input.variables > 8
        || input.rounds.is_empty()
        || input.rounds.len() > 16
        || input.clauses.len()
            + input
                .rounds
                .iter()
                .map(|round| round.add.len())
                .sum::<usize>()
            > 256
    {
        return Err("evaluation bounds exceeded".into());
    }
    let mut solver = Solver::new();
    solver.enable_self_checking();
    for _ in 0..input.variables {
        solver.new_var();
    }
    let mut clauses = input.clauses;
    for clause in &clauses {
        solver.add_clause(&literals(clause, input.variables)?);
    }
    let mut results = Vec::new();
    for round in input.rounds {
        for clause in round.add {
            solver.add_clause(&literals(&clause, input.variables)?);
            clauses.push(clause);
        }
        solver.assume(&literals(&round.assumptions, input.variables)?);
        let sat = solver.solve().map_err(|error| error.to_string())?;
        if sat {
            let mut model = vec![false; input.variables];
            for literal in solver.model().ok_or("SAT without model")? {
                model[literal.var().index()] = literal.is_positive();
            }
            let satisfies = |clause: &[i32]| {
                clause
                    .iter()
                    .any(|literal| model[literal.unsigned_abs() as usize - 1] == (*literal > 0))
            };
            if !clauses.iter().all(|clause| satisfies(clause))
                || !round
                    .assumptions
                    .iter()
                    .all(|literal| satisfies(&[*literal]))
            {
                return Err("returned SAT model does not satisfy input".into());
            }
            results.push(json!({"status":"sat","model":model}));
        } else {
            results.push(json!({"status":"unsat","model":null}));
        }
    }
    Ok(json!({"kind":"results","results":results,"selfChecking":true,"workBudget":false}))
}
