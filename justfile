default:
  @just --list

sat-build:
  cargo build --release --locked --manifest-path evals/sat-foundation/rust/Cargo.toml
  cargo build --release --locked --lib --target wasm32-unknown-unknown --manifest-path evals/sat-foundation/rust/Cargo.toml

sat-test: sat-build
  cargo fmt --manifest-path evals/sat-foundation/rust/Cargo.toml -- --check
  cargo clippy --all-targets --locked --manifest-path evals/sat-foundation/rust/Cargo.toml -- -D warnings
  cargo test --all-targets --locked --manifest-path evals/sat-foundation/rust/Cargo.toml
  pnpm build
  node --test dist/evals/sat-foundation/contract-test.js

sat-browser: sat-build
  pnpm exec playwright test --config sat-playwright.config.ts

sat-eval phase run parent="": sat-build
  pnpm build
  node dist/evals/sat-foundation/run.js {{quote(phase)}} {{quote(run)}} {{quote(parent)}}

sat-replay run:
  pnpm build
  node dist/evals/sat-foundation/replay.js {{quote(run)}}

testing-build:
  cargo build --release --locked --manifest-path crates/idealis-testing/Cargo.toml
  cargo build --release --locked --lib --target wasm32-unknown-unknown --manifest-path crates/idealis-testing/Cargo.toml

testing-core-test:
  cargo fmt --manifest-path crates/idealis-testing/Cargo.toml -- --check
  cargo clippy --all-targets --locked --manifest-path crates/idealis-testing/Cargo.toml -- -D warnings
  cargo test --all-targets --locked --manifest-path crates/idealis-testing/Cargo.toml

testing-example:
  cargo run --example counter --locked --manifest-path crates/idealis-testing/Cargo.toml

testing-test: testing-build
  pnpm build
  node --test dist/evals/testing-core/contract-test.js
  pnpm exec vitest run --config testing-vitest.config.ts

testing-browser: testing-build
  pnpm exec playwright test --config testing-playwright.config.ts

testing-eval phase run parent="": testing-build
  pnpm build
  node dist/evals/testing-core/run.js {{quote(phase)}} {{quote(run)}} {{quote(parent)}}

testing-replay run:
  pnpm build
  node dist/evals/testing-core/replay.js {{quote(run)}}

ralloy-build:
  cargo build --release --locked --manifest-path crates/ralloy/Cargo.toml
  cargo build --release --locked --lib --target wasm32-unknown-unknown --manifest-path crates/ralloy/Cargo.toml

ralloy-test:
  cargo fmt --manifest-path crates/ralloy/Cargo.toml -- --check
  cargo clippy --all-targets --locked --manifest-path crates/ralloy/Cargo.toml -- -D warnings
  cargo test --all-targets --locked --manifest-path crates/ralloy/Cargo.toml

ralloy-example:
  cargo run --example graph --locked --manifest-path crates/ralloy/Cargo.toml

ralloy-contract: ralloy-build
  pnpm build
  node --test dist/evals/alloy-subset/contract-test.js

ralloy-browser: ralloy-build
  pnpm exec playwright test --config alloy-playwright.config.ts

ralloy-eval phase run java jar parent="": ralloy-build
  pnpm build
  node dist/evals/alloy-subset/run.js {{quote(phase)}} {{quote(run)}} {{quote(java)}} {{quote(jar)}} {{quote(parent)}}

ralloy-replay run:
  pnpm build
  node dist/evals/alloy-subset/replay.js {{quote(run)}}

alloy-test: ralloy-build
  pnpm build
  node --test dist/evals/alloy-instance/contract-test.js
  pnpm exec vitest run --config alloy-vitest.config.ts

alloy-browser: ralloy-build
  pnpm exec playwright test --config alloy-instance-playwright.config.ts

alloy-instance-eval phase run java jar parent="": ralloy-build
  pnpm build
  node dist/evals/alloy-instance/run.js {{quote(phase)}} {{quote(run)}} {{quote(java)}} {{quote(jar)}} {{quote(parent)}}

alloy-instance-replay run:
  pnpm build
  node dist/evals/alloy-instance/replay.js {{quote(run)}}

rsmt-build:
  pnpm rsmt:build

rsmt-test:
  pnpm rsmt:test

rsmt-example:
  cargo run --example ttl --locked --manifest-path crates/idealis-smt/Cargo.toml

rsmt-compare run baseline: rsmt-build
  pnpm build
  node dist/evals/rsmt-extraction/compare.js check {{quote(run)}} {{quote(baseline)}}

rsmt-criterion:
  cargo bench --locked --manifest-path crates/idealis-smt/Cargo.toml --bench solver

smt-build: rsmt-build

smt-test: rsmt-test

smt-criterion: rsmt-criterion

smt-verify phase run parent="": smt-build
  pnpm build
  node dist/evals/smt-subset/verify-run.js {{quote(phase)}} {{quote(run)}} {{quote(parent)}}

smt-bench run z3: smt-build
  pnpm build
  node dist/evals/smt-subset/benchmark.js {{quote(run)}} {{quote(z3)}}

smt-browser: smt-build
  pnpm exec playwright test --config smt-playwright.config.ts

install:
  pnpm install --frozen-lockfile

check:
  pnpm check
  pnpm typecheck
  pnpm test

test:
  pnpm test

mutation:
  pnpm test:mutation

browser: portable-build
  pnpm test:browser

demo:
  pnpm example

evaluate:
  pnpm evaluate

replay-evaluation:
  pnpm evaluate:replay

eval-loop:
  pnpm eval:loop evals/plans/0002-mixed-depth.json

replay-loop:
  pnpm eval:replay

runtime-build:
  pnpm runtime:build

runtime-test:
  pnpm runtime:test
  pnpm runtime:browser

runtime-eval:
  pnpm runtime:eval

portable-build:
  pnpm portable:build

portable-test:
  pnpm portable:test
  pnpm portable:browser

portable-eval:
  pnpm portable:eval

portable-replay run:
  pnpm portable:replay {{quote(run)}}

safety-build: portable-build
  pnpm build

safety-test: safety-build
  node --test dist/evals/rust-safety/contract-test.js dist/evals/collections-temporal/contract-test.js dist/evals/tlc-differential/compare-test.js dist/evals/tlc-differential/regression-test.js

safety-eval run plan="0014-value-hash": safety-build
  node dist/evals/rust-safety/evaluate.js {{quote(run)}} {{quote(plan)}}

safety-replay run:
  pnpm build
  node dist/evals/rust-safety/replay.js {{quote(run)}}

safety-tlc java jar: safety-build
  node dist/evals/rust-safety/tlc-check.js {{quote(java)}} {{quote(jar)}}

go-test: portable-build
  pnpm build
  node dist/evals/go/prepare.js
  test -z "$(gofmt -l adapters/go)"
  cd adapters/go && go vet ./...
  cd adapters/go && CGO_ENABLED=0 go test -count=1 ./...
  cd adapters/go && go test -race -count=1 ./...
  node dist/scripts/go-consumer.js

cross-build: portable-build
  pnpm build
  cargo build --release --locked --manifest-path evals/shared-oracle-rust/Cargo.toml
  mkdir -p .artifacts/go
  cd adapters/go && CGO_ENABLED=0 go build -o ../../.artifacts/go/counter-replay ./cmd/counter-replay

cross-test: cross-build
  cargo fmt --manifest-path evals/shared-oracle-rust/Cargo.toml -- --check
  cargo clippy --all-targets --locked --manifest-path evals/shared-oracle-rust/Cargo.toml -- -D warnings
  node --test dist/evals/cross-language/contract-test.js

cross-eval phase run parent="": cross-build
  node dist/evals/cross-language/evaluate.js {{quote(phase)}} {{quote(run)}} {{quote(parent)}}

cross-replay run:
  pnpm build
  node dist/evals/cross-language/replay.js {{quote(run)}}

async-build: cross-build
  cd adapters/go && CGO_ENABLED=0 go build -o ../../.artifacts/go/history-check ./cmd/history-check
  cd adapters/go && CGO_ENABLED=0 go build -o ../../.artifacts/go/finite-bench ./cmd/finite-bench

async-test: async-build
  node --test dist/evals/async-tuning/contract-test.js

async-eval phase run parent="": async-build
  node dist/evals/async-tuning/evaluate.js {{quote(phase)}} {{quote(run)}} {{quote(parent)}}

async-replay run:
  pnpm build
  node dist/evals/async-tuning/replay.js {{quote(run)}}

usable-test:
  pnpm usable:test

usable-eval:
  pnpm usable:eval

usable-replay run:
  pnpm usable:replay {{quote(run)}}

usable-benchmark input output:
  pnpm build
  node dist/evals/usable/benchmark-run.js {{quote(input)}} {{quote(output)}}

package:
  pnpm package:prepare
  pnpm package:test

complex-tlc java jar run: safety-build
  node dist/evals/collections-temporal/tlc-check.js {{quote(java)}} {{quote(jar)}} {{quote(run)}}

complex-eval run parent="": safety-build
  node dist/evals/collections-temporal/evaluate.js {{quote(run)}} {{quote(parent)}}

complex-replay run:
  pnpm build
  node dist/evals/collections-temporal/replay.js {{quote(run)}}

complex-tlc-replay java jar run:
  pnpm build
  node dist/evals/collections-temporal/tlc-replay.js {{quote(java)}} {{quote(jar)}} {{quote(run)}}


tlc-diff java jar phase run: safety-build
  node dist/evals/tlc-differential/run.js {{quote(java)}} {{quote(jar)}} {{quote(phase)}} {{quote(run)}}

tlc-diff-replay java jar saved run: safety-build
  node dist/evals/tlc-differential/run.js {{quote(java)}} {{quote(jar)}} replay {{quote(run)}} {{quote(saved)}}

checker-bench-test:
  pnpm build
  node --test dist/evals/checker-benchmark/contract-test.js

checker-bench java jar archive run: safety-build
  node dist/evals/checker-benchmark/run.js {{quote(java)}} {{quote(jar)}} {{quote(archive)}} {{quote(run)}}

checker-bench-replay java jar archive saved run: safety-build
  node dist/evals/checker-benchmark/replay.js {{quote(java)}} {{quote(jar)}} {{quote(archive)}} {{quote(saved)}} {{quote(run)}}

parallel-safety-test: safety-build
  node --test dist/evals/parallel-safety/contract-test.js

parallel-safety-eval run: safety-build
  node dist/evals/parallel-safety/evaluate.js {{quote(run)}}

parallel-safety-replay saved run: safety-build
  node dist/evals/parallel-safety/replay.js {{quote(saved)}} {{quote(run)}}

tlc-throughput java jar archive run: safety-build
  node dist/evals/tlc-throughput/run.js {{quote(java)}} {{quote(jar)}} {{quote(archive)}} {{quote(run)}}

tlc-throughput-replay java jar archive saved run: safety-build
  node dist/evals/tlc-throughput/run.js {{quote(java)}} {{quote(jar)}} {{quote(archive)}} {{quote(run)}} {{quote(saved)}}

single-model-test java jar: safety-build
  IDEALIS_TLC_JAVA={{quote(java)}} IDEALIS_TLC_JAR={{quote(jar)}} node --test dist/evals/single-model/contract-test.js dist/evals/single-model/integration-test.js

single-model-eval java jar archive run: safety-build
  node dist/evals/single-model/evaluate.js {{quote(java)}} {{quote(jar)}} {{quote(archive)}} {{quote(run)}}

single-model-replay java jar archive saved run: safety-build
  node dist/evals/single-model/evaluate.js {{quote(java)}} {{quote(jar)}} {{quote(archive)}} {{quote(run)}} {{quote(saved)}}

frontier-safety-test: safety-build
  cargo test --all-targets --locked --manifest-path crates/idealis-core/Cargo.toml
  node --test dist/evals/frontier-safety/contract-test.js

frontier-safety-eval java jar archive run: safety-build
  node dist/evals/frontier-safety/evaluate.js {{quote(java)}} {{quote(jar)}} {{quote(archive)}} {{quote(run)}}

frontier-safety-replay java jar archive saved run: safety-build
  node dist/evals/frontier-safety/evaluate.js {{quote(java)}} {{quote(jar)}} {{quote(archive)}} {{quote(run)}} {{quote(saved)}}

frontier-budget-eval java jar archive run: safety-build
  IDEALIS_FRONTIER_PLAN=0021-worker-budget node dist/evals/frontier-safety/evaluate.js {{quote(java)}} {{quote(jar)}} {{quote(archive)}} {{quote(run)}}

frontier-budget-replay java jar archive saved run: safety-build
  IDEALIS_FRONTIER_PLAN=0021-worker-budget node dist/evals/frontier-safety/evaluate.js {{quote(java)}} {{quote(jar)}} {{quote(archive)}} {{quote(run)}} {{quote(saved)}}

readme-bench-test java jar: safety-build
  IDEALIS_TLC_JAVA={{quote(java)}} IDEALIS_TLC_JAR={{quote(jar)}} node --test dist/evals/readme-comparison/contract-test.js dist/evals/readme-comparison/integration-test.js

readme-bench java jar archive run: safety-build
  node dist/evals/readme-comparison/evaluate.js {{quote(java)}} {{quote(jar)}} {{quote(archive)}} {{quote(run)}}

readme-bench-replay java jar archive saved run: safety-build
  node dist/evals/readme-comparison/evaluate.js {{quote(java)}} {{quote(jar)}} {{quote(archive)}} {{quote(run)}} {{quote(saved)}}

criterion-prepare:
  pnpm build
  node dist/evals/criterion-safety/prepare.js

criterion-test: criterion-prepare
  cargo test --locked --manifest-path crates/rtlc/Cargo.toml --bench safety
  node --test dist/evals/criterion-safety/contract-test.js

criterion-save stage run: criterion-prepare
  node dist/evals/criterion-safety/run.js {{quote(stage)}} {{quote(run)}}

criterion-controls run: safety-build
  node dist/evals/criterion-safety/compare.js {{quote(run)}} controls

criterion-compare run: safety-build
  node dist/evals/criterion-safety/compare.js {{quote(run)}}

criterion-replay saved run:
  pnpm build
  node dist/evals/criterion-safety/compare.js {{quote(run)}} {{quote(saved)}}

profile-criterion stage run: criterion-prepare
  IDEALIS_CRITERION_PLAN=0024-profile-tuning node dist/evals/criterion-safety/run.js {{quote(stage)}} {{quote(run)}}

profile-record backend binary run:
  pnpm build
  node dist/evals/profile-tuning/record.js {{quote(backend)}} {{quote(binary)}} {{quote(run)}}

profile-test: criterion-prepare
  node --test dist/evals/profile-tuning/hotspots-test.js dist/evals/profile-tuning/wasm-names-test.js

profile-summary saved run:
  pnpm build
  node dist/evals/profile-tuning/summarize.js {{quote(saved)}} {{quote(run)}}

profile-scalar run: safety-build
  node dist/evals/profile-tuning/scalar-compare.js {{quote(run)}}

profile-replay saved run:
  pnpm build
  node dist/evals/profile-tuning/scalar-compare.js {{quote(run)}} {{quote(saved)}}

rtlc-test:
  pnpm rtlc:test

rtlc-example:
  cargo run --example counter --locked --manifest-path crates/rtlc/Cargo.toml

rtlc-compare run:
  pnpm build
  node dist/evals/rtlc-extraction/compare.js {{quote(run)}}

rtla-build:
  pnpm rtla:build

compiler-test:
  cargo fmt --manifest-path crates/rtla-compiler/Cargo.toml -- --check
  cargo clippy --all-targets --locked --manifest-path crates/rtla-compiler/Cargo.toml -- -D warnings
  cargo test --all-targets --locked --manifest-path crates/rtla-compiler/Cargo.toml

compiler-eval run: portable-build
  pnpm build
  node dist/evals/rtla-compiler/evaluate.js {{quote(run)}}

compiler-tlc run java jar: portable-build
  pnpm build
  node dist/evals/rtla-compiler/tlc.js {{quote(run)}} {{quote(java)}} {{quote(jar)}}

rtla-test:
  pnpm rtla:test

rtla-browser: rtla-build
  pnpm exec playwright test --config rtla-playwright.config.ts

rtla-eval run java jar: rtla-build
  pnpm build
  node dist/evals/rtla-parser/evaluate.js {{quote(run)}} {{quote(java)}} {{quote(jar)}}

apalache-test:
  pnpm build
  node --test dist/evals/apalache-comparison/contract-test.js

apalache-eval run java jar: portable-build
  pnpm build
  node dist/evals/apalache-comparison/run.js {{quote(run)}} {{quote(java)}} {{quote(jar)}}

backend-test: portable-build
  pnpm build
  pnpm exec vitest run tests/backend.vitest.test.ts
  node --test dist/evals/backend-contract/integration-test.js

backend-eval run: portable-build
  pnpm build
  node dist/evals/backend-contract/evaluate.js {{quote(run)}}
