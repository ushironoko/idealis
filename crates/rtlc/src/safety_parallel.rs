use super::Search;
use crate::CoreError;
use crate::ir::{SafetyProgram, Scalar, Step};
use crate::safety_expressions::{EvalContext, visit_branch};
use crate::safety_values::{Arena, ValueDelta};
use crate::safety_work::WorkBudget;
use rayon::iter::{
    IndexedParallelIterator, IntoParallelIterator, IntoParallelRefMutIterator, ParallelIterator,
};
use std::ops::Range;

enum Event {
    Candidate {
        parent: usize,
        action: usize,
        start: usize,
    },
    Deadlock(usize),
    Error(CoreError),
}

#[derive(Default)]
struct Candidates {
    events: Vec<Event>,
    values: Vec<Option<Scalar>>,
}

impl Candidates {
    fn clear(&mut self) {
        self.events.clear();
        self.values.clear();
    }
    fn push(&mut self, parent: usize, action: usize, values: &[Option<Scalar>]) {
        self.events.push(Event::Candidate {
            parent,
            action,
            start: self.values.len(),
        });
        self.values.extend_from_slice(values);
    }
}

struct Buffer {
    candidates: Candidates,
    next: Vec<Option<Scalar>>,
}

impl Buffer {
    fn new(width: usize) -> Self {
        Self {
            candidates: Candidates::default(),
            next: vec![None; width],
        }
    }
    fn reset(&mut self) {
        self.candidates.clear();
        self.next.fill(None);
    }
}

pub(super) struct Workers {
    pool: rayon::ThreadPool,
    buffers: Vec<Buffer>,
    chunk: usize,
}

impl Workers {
    pub(super) fn new(workers: usize, width: usize) -> Result<Self, CoreError> {
        Ok(Self {
            pool: rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .map_err(|error| CoreError::protocol(&format!("Safety worker pool: {error}")))?,
            buffers: (0..workers).map(|_| Buffer::new(width)).collect(),
            chunk: 64,
        })
    }
}

impl Workers {
    fn observe(&mut self, parents: usize, candidates: usize) {
        self.chunk = (1024 * parents / candidates.max(1)).clamp(16, 128);
    }
}

pub(super) fn is_expensive(program: &SafetyProgram) -> bool {
    program
        .next
        .iter()
        .flat_map(|branch| &branch.steps)
        .any(|step| match step {
            Step::Assign { value, .. } => !value.is_scalar_cost(),
            Step::Guard { expression } => !expression.is_scalar_cost(),
            Step::Bind { .. } | Step::Choose { .. } | Step::Let { .. } => true,
        })
        || program
            .invariants
            .iter()
            .any(|predicate| !predicate.expression.is_scalar_cost())
}

impl Search<'_> {
    pub(super) fn parallel_frontier(
        &mut self,
        frontier: Range<usize>,
        workers: &mut Workers,
        arena: &mut Arena,
    ) -> Result<(), CoreError> {
        let mut start = frontier.start;
        while start < frontier.end {
            let active = workers
                .buffers
                .len()
                .min((frontier.end - start).div_ceil(8));
            let chunk = (frontier.end - start).div_ceil(active).min(workers.chunk);
            let end = (start + chunk * active).min(frontier.end);
            let ranges: Vec<_> = (start..end)
                .step_by(chunk)
                .map(|first| first..(first + chunk).min(end))
                .collect();
            let budget = WorkBudget::new(self.bounds.max_work - arena.work, ranges.len());
            let expansions: Vec<_> = workers.pool.install(|| {
                ranges
                    .into_par_iter()
                    .zip(workers.buffers.par_iter_mut())
                    .enumerate()
                    .map(|(worker, (range, buffer))| {
                        self.expand_range(range, arena, &budget, worker, buffer)
                    })
                    .collect()
            });
            arena.work += expansions.iter().map(|delta| delta.work).sum::<u32>();
            let candidates = workers
                .buffers
                .iter()
                .take(expansions.len())
                .map(|buffer| buffer.candidates.values.len() / self.store.width)
                .sum();
            workers.observe(end - start, candidates);
            for (delta, buffer) in expansions.into_iter().zip(&mut workers.buffers) {
                let mapping = arena.merge(delta)?;
                for value in buffer.candidates.values.iter_mut().flatten() {
                    *value = mapping.get(*value);
                }
                for event in buffer.candidates.events.drain(..) {
                    match event {
                        Event::Candidate {
                            parent,
                            action,
                            start,
                        } => {
                            self.accept(
                                &buffer.candidates.values[start..start + self.store.width],
                                Some(parent),
                                action,
                                arena,
                            )?;
                        }
                        Event::Deadlock(parent) => self.deadlock(parent, arena)?,
                        Event::Error(error) => return Err(error),
                    }
                    if self.stopped() {
                        return Ok(());
                    }
                }
            }
            start = end;
        }
        Ok(())
    }

    fn expand_range(
        &self,
        range: Range<usize>,
        base: &Arena,
        budget: &WorkBudget,
        worker: usize,
        buffer: &mut Buffer,
    ) -> ValueDelta {
        buffer.reset();
        let mut arena = base.fork(budget, worker);
        for parent in range {
            let current = self.store.state(parent);
            let mut enabled = false;
            for (action, branch) in self.program.next.iter().enumerate() {
                let result = visit_branch(
                    branch,
                    0,
                    EvalContext::new(current, &[]),
                    &mut buffer.next,
                    &mut arena,
                    &mut |values, _arena| {
                        enabled = true;
                        buffer.candidates.push(parent, action, values);
                        Ok(false)
                    },
                );
                match result {
                    Ok(true) => return arena.into_delta(),
                    Ok(false) => {}
                    Err(error) => {
                        buffer.candidates.events.push(Event::Error(error));
                        return arena.into_delta();
                    }
                }
            }
            if !enabled && self.program.check_deadlock {
                buffer.candidates.events.push(Event::Deadlock(parent));
                break;
            }
        }
        arena.into_delta()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_keep_snapshots_and_order_when_buffers_are_reused() {
        let mut buffer = Candidates::default();
        let mut values = [Some(Scalar::Int(1)), Some(Scalar::Int(2))];
        buffer.push(3, 0, &values);
        values[0] = Some(Scalar::Int(7));
        buffer.push(4, 1, &values);
        assert_eq!(
            buffer.values,
            [
                Some(Scalar::Int(1)),
                Some(Scalar::Int(2)),
                Some(Scalar::Int(7)),
                Some(Scalar::Int(2))
            ]
        );
        for (event, expected) in buffer.events.iter().zip([(3, 0, 0), (4, 1, 2)]) {
            let Event::Candidate {
                parent,
                action,
                start,
                ..
            } = event
            else {
                panic!("expected candidate")
            };
            assert_eq!((*parent, *action, *start), expected);
        }
        buffer.clear();
        buffer.push(5, 2, &values);
        assert_eq!(buffer.values, values);
        assert_eq!(buffer.events.len(), 1);
    }
}

#[cfg(test)]
mod schedule_tests {
    use super::*;

    #[test]
    fn batch_tracks_fanout_and_stays_bounded() {
        let mut workers = Workers::new(2, 1).unwrap();
        assert_eq!(workers.chunk, 64);
        workers.observe(64, 2048);
        assert_eq!(workers.chunk, 32);
        workers.observe(64, 1);
        assert_eq!(workers.chunk, 128);
        workers.observe(64, 100_000);
        assert_eq!(workers.chunk, 16);
        workers.observe(64, 0);
        assert_eq!(workers.chunk, 128);
    }
}
