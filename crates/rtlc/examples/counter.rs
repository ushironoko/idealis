use rtlc::SafetyProgram;
use rtlc::ir::{Branch, Expression, Predicate, Scalar, Step};

fn integer(value: i32) -> Expression {
    Expression::Literal {
        value: Scalar::Int(value),
    }
}
fn read() -> Expression {
    Expression::Read {
        slot: 0,
        next: false,
    }
}
pub fn ring() -> SafetyProgram {
    SafetyProgram {
        format: "modelt.safety-program".into(),
        version: 1,
        name: "Ring".into(),
        variables: vec!["x".into()],
        initial: vec![Branch {
            name: "Init".into(),
            steps: vec![Step::Assign {
                slot: 0,
                value: integer(0),
            }],
        }],
        next: vec![Branch {
            name: "Next".into(),
            steps: vec![Step::Assign {
                slot: 0,
                value: Expression::If {
                    condition: Box::new(Expression::Lt {
                        left: Box::new(read()),
                        right: Box::new(integer(3)),
                    }),
                    consequent: Box::new(Expression::Add {
                        left: Box::new(read()),
                        right: Box::new(integer(1)),
                    }),
                    alternative: Box::new(integer(0)),
                },
            }],
        }],
        invariants: vec![Predicate {
            name: "Safe".into(),
            expression: Expression::Le {
                left: Box::new(read()),
                right: Box::new(integer(3)),
            },
        }],
        assumptions: vec![],
        check_deadlock: true,
        initial_properties: vec![],
        action_properties: vec![],
        properties: vec![],
        fairness: vec![],
        operators: vec![],
    }
}
#[cfg(not(test))]
fn main() {
    let result = ring()
        .check(rtlc::SafetyBounds {
            max_depth: 10,
            max_states: 100,
            max_work: 1000,
            max_temporal_nodes: None,
        })
        .unwrap();
    assert_eq!(result.status, "complete");
    assert_eq!((result.states, result.generated), (4, 5));
    println!("{}", serde_json::to_string(&result).unwrap());
}
