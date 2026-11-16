import type { Input } from "./corpus.js";

const satisfies = (
  clauses: readonly (readonly number[])[],
  assumptions: readonly number[],
  model: readonly boolean[],
) =>
  [...clauses, ...assumptions.map((literal) => [literal])].every((clause) =>
    clause.some((literal) => model[Math.abs(literal) - 1] === literal > 0),
  );

export const enumerate = (input: Input): readonly string[] => {
  const clauses = [...input.clauses];
  return input.rounds.map((round) => {
    clauses.push(...round.add);
    for (let bits = 0; bits < 2 ** input.variables; bits++) {
      const model = Array.from(
        { length: input.variables },
        (_, index) => (bits & (1 << index)) !== 0,
      );
      if (satisfies(clauses, round.assumptions, model)) return "sat";
    }
    return "unsat";
  });
};

export const validate = (input: Input, value: unknown): readonly string[] => {
  if (
    typeof value !== "object" ||
    value === null ||
    !("kind" in value) ||
    value.kind !== "results" ||
    !("selfChecking" in value) ||
    value.selfChecking !== true ||
    !("workBudget" in value) ||
    value.workBudget !== false ||
    !("results" in value) ||
    !Array.isArray(value.results) ||
    value.results.length !== input.rounds.length
  )
    throw new Error(`Invalid solver reply: ${JSON.stringify(value)}`);
  const expected = enumerate(input);
  const clauses = [...input.clauses];
  const answers = value.results;
  return input.rounds.map((round, index) => {
    clauses.push(...round.add);
    const answer: unknown = answers[index];
    if (
      typeof answer !== "object" ||
      answer === null ||
      !("status" in answer) ||
      answer.status !== expected[index] ||
      !("model" in answer)
    )
      throw new Error(`Wrong SAT status in round ${index}`);
    if (answer.status === "unsat") {
      if (answer.model !== null)
        throw new Error("UNSAT must not return a model");
    } else if (
      !Array.isArray(answer.model) ||
      answer.model.length !== input.variables ||
      !answer.model.every((bit: unknown) => typeof bit === "boolean") ||
      !satisfies(clauses, round.assumptions, answer.model)
    )
      throw new Error(`Invalid SAT witness in round ${index}`);
    return String(answer.status);
  });
};
