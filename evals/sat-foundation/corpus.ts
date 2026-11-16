export interface Round {
  readonly add: readonly (readonly number[])[];
  readonly assumptions: readonly number[];
}
export interface Input {
  readonly variables: number;
  readonly clauses: readonly (readonly number[])[];
  readonly rounds: readonly Round[];
}

export const fixed: readonly Input[] = [
  { variables: 0, clauses: [], rounds: [{ add: [], assumptions: [] }] },
  { variables: 0, clauses: [[]], rounds: [{ add: [], assumptions: [] }] },
  { variables: 8, clauses: [], rounds: [{ add: [], assumptions: [] }] },
  {
    variables: 2,
    clauses: [[1, 2]],
    rounds: [
      { add: [], assumptions: [-1] },
      { add: [], assumptions: [-1, -2] },
      { add: [], assumptions: [] },
      { add: [[-1], [-2]], assumptions: [] },
    ],
  },
  {
    variables: 2,
    clauses: [
      [1, 1],
      [-2, 2],
    ],
    rounds: [{ add: [], assumptions: [] }],
  },
  {
    variables: 1,
    clauses: [],
    rounds: [
      { add: [], assumptions: [1, -1] },
      { add: [], assumptions: [] },
    ],
  },
  {
    variables: 2,
    clauses: [
      [-1, 2],
      [1, -2],
    ],
    rounds: [
      { add: [], assumptions: [1] },
      { add: [[-1]], assumptions: [] },
    ],
  },
];

export const createCorpus = (seed: number, count: number): readonly Input[] => {
  if (
    !Number.isInteger(seed) ||
    seed <= 0 ||
    seed > 0xffffffff ||
    count < fixed.length
  )
    throw new Error("Invalid corpus parameters");
  let state = seed >>> 0;
  const random = (bound: number) => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>> 0) % bound;
  };
  return [
    ...fixed,
    ...Array.from({ length: count - fixed.length }, (_, index): Input => {
      const variables = index % 9;
      const mode = Math.floor(index / 9) % 3;
      const planted = Array.from({ length: variables }, () => random(2) === 1);
      const literal = () =>
        (random(variables) + 1) * (random(2) === 1 ? 1 : -1);
      const clauses =
        variables === 0
          ? []
          : Array.from({ length: 2 + random(17) }, () => {
              const clause = Array.from({ length: 1 + random(4) }, literal);
              if (mode === 1) {
                const variable = random(variables);
                clause[0] = (variable + 1) * (planted[variable] ? 1 : -1);
              }
              return clause;
            });
      if (mode === 2) clauses.push(...(variables === 0 ? [[]] : [[1], [-1]]));
      if (variables > 0 && index % 4 === 0) clauses.push([1, -1], [1, 1, -1]);
      return {
        variables,
        clauses,
        rounds: [
          { add: [], assumptions: [] },
          {
            add: [],
            assumptions:
              variables === 0 ? [] : index % 2 === 0 ? [1, -1] : [literal()],
          },
          {
            add:
              variables === 0 ? (index % 2 === 0 ? [[]] : []) : [[literal()]],
            assumptions: [],
          },
        ],
      };
    }),
  ];
};
