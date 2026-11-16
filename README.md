# idealis

**このリポジトリは開発途中の未完成なツールキットです。** 現在は `0.1.0-alpha.0` の試作段階で、API・対応範囲・backend間の統合を整備しています。npm / crates.io には未公開です。

形式モデルを基準に、モデル検査と実装の適合性検査を行うポータブルなツールキット。モデルからのオラクル生成、差分テスト、反例を縮小する property-based testing を、Vitest・`node:test`・ブラウザで実行する。

CLI は `idealis`、npm package は `@mizchi/idealis`。Rust のホストは `idealis-core`、SMT サブセットは `idealis-smt`。[旧名からの移行と保存形式の互換性](docs/migration.md)を参照。

## 構成とライブラリの役割

| ライブラリ                                      | 入力                                               | 担当する処理・出力                                                                                                 |
| ----------------------------------------------- | -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| [`rtla`](crates/rtla)                           | TLA+ソースとTLC設定                                | 構文解析し、構文木（AST）を返す。名前解決やモデル検査は後段が担当する。                                            |
| [`rtla-compiler`](crates/rtla-compiler)         | rtlaのAST                                          | 名前解決、定数・operatorの展開を行い、対応する有限モデルの中間表現（IR）へ変換する。                               |
| [`rtlc`](crates/rtlc)                           | 検査用IR                                           | 状態を列挙し、invariant・deadlock・temporal property・fairnessを検査して反例を返す。nativeの探索並列化も担当する。 |
| [`idealis-core`](crates/idealis-core)           | TLA+ソース、IR、有限モデル、実装の観測・履歴       | parser・compiler・checkerを接続し、実装照合、履歴検査、再生、縮小候補の生成とnative JSONL / Wasm境界を提供する。   |
| [`idealis-smt`](crates/idealis-smt)（旧`rsmt`） | 対応するSMT-LIB式                                  | 任意精度整数の差分制約とBoolean式を解き、SATの具体値、UNSAT、予算切れのunknownを返す独立した試作ソルバー。         |
| [`ralloy`](crates/ralloy)                       | 静的なAlloyソースと有限scope                       | sig・二項関係・fact・assertを解析し、構造を列挙してinstanceや反例を返す。native/Wasmで動く独立した試作。           |
| [`idealis-testing`](crates/idealis-testing)     | 有限生成器、保存case、評価feedback、複数実装の観測 | Rustで生成・縮小・差分比較・再生を管理し、native/Wasmのeffectを各言語のDriverが評価する。                          |
| [`@mizchi/idealis`](src)                        | TSのモデルとDriver、各backendの入出力              | オラクル生成、実装との照合、差分・property-based test、反例縮小、スケジューリングとCLIを提供する。                 |
| [Go adapter](adapters/go)                       | 共通コアのWasmとGoのモデル・実装・履歴             | CGOなしでRustコアへ接続し、同じ値・オラクル・観測を使って照合と再生を行う。                                        |

RustのTLA+経路は、`TLA+ソース → rtlaのAST → rtla-compilerのIR → rtlcの探索・検査`。`idealis-core`がこの経路をnative/Wasmのリクエストへ接続する。TSはRust frontend APIから利用し、Go adapterは共通コアの有限モデル・履歴照合を利用する。rtlaとrtlcはそれぞれ単独で利用でき、idealis-coreへ依存しない。

`idealis-smt`はこのTLA+探索経路から独立している。現在の公開SMT adapterはZ3 Wasmを使い、idealis-smtを公開backendへ組み込む作業は今後の対象。

## 対応範囲と統合状況

TLA+・SMT-LIB・Alloyの対応はサブセットに限られる。モデル検査が確認するのは、指定したモデル・仮定・探索範囲に対する性質。実装との照合は、実行した操作列と観測を対象にする。予算内で反例が見つからなかった結果や、実装との一致を、全入力・全実行の正しさの証明へ読み替えない。[検証範囲](#検証の範囲)と各crateの制約を確認する。

TLA+はRust/Wasm frontendとtla-ts経路を持ち、`z3-solver`を任意のSMT adapterとして利用する。Quint Connectのモデルと実装をつなぐアプローチを参考に、TSの`Driver`を実装との境界にした。Quint言語やApalacheの再実装ではない。

[独立した`idealis-smt` crate](crates/idealis-smt)は、整数の差分制約とBoolean式をnative/Wasmで解く。[対応するSMT-LIBサブセット](docs/smt-subset.md)を明示し、idealisの他crateやZ3には依存しない。Z3を再利用した比較でも小さいTTL・amount・schedulerはnativeで約3.0-3.4倍速い一方、Boolean UNSATには後退と予算切れが残る。[全17入力の比較](evals/iterations/0031-smt-reuse.md)は切り出し前の測定で、idealisの公開backendへの組み込みは保留している。

[`ralloy`](crates/ralloy)は、静的なsig・二項field・関係演算・量化・run/checkを有限scope内で検査する独立crate。JVMやZ3に依存せず、nativeとホストimportのないWasmで同じ探索器を使う。Alloy AnalyzerのSAT変換、継承、時間・var、整数演算は未対応。`@mizchi/idealis/alloy`からWasmを読み、モデルの探索と実装の関係スナップショットの照合に使える。[利用手順](docs/alloy.md)と[評価手順](evals/alloy-subset/README.md)を参照。汎用backend契約とCLIへの接続は今後の対象。

[`idealis-testing`](crates/idealis-testing)は、有限状態機械のPBT・縮小・差分比較・保存caseの再生を行う独立crate。TSは`@mizchi/idealis/testing`から同じWasmを使い、Vitest・node:test・ブラウザでDriverを動かす。[利用手順](docs/testing.md)を参照。既存fast-check経路のStrategy/seed互換、並行スケジュール探索、全実行の証明は含めない。

[SAT/CNF候補の評価](evals/iterations/0037-sat-foundation.md)では、Varisat 0.2.2のnative/Wasmを独立全列挙と照合し、実ブラウザの動作を確認した。実行バジェットを保証するbackendへの採用は保留中。`ralloy`や`idealis-smt`には組み込まず、非公開のeval crateとして分離している。

## 実行

観測付きの有限モデルを TS / Rust で共有する方法、反例の縮小、時計・通信・永続化の注入は [利用ガイド](docs/using-models.md) を参照。`just package` で solver を必須にしない npm/Cargo/Wasm archive と CLI を作り、別 consumer で検証できる。[公開手順と CI の分割](docs/releasing.md) は別途記載している。

Node.js 24+、pnpm、just を使う。リポジトリで Rust backend とブラウザの開発検査を実行する場合は Rust と `wasm32-unknown-unknown` target も必要。配布 Wasm の利用側で Rust をビルドする必要はない。npm には未公開。

```sh
pnpm install --frozen-lockfile
just check
just demo
just mutation
pnpm exec playwright install chromium
rustup target add wasm32-unknown-unknown
just browser
```

`just check` は lint、型検査、Vitest、`node:test` を実行する。共通の挙動テストを両ランナーに登録している。`just mutation` は `@mizchi/mutator` で TS ソースを変異させ、Vitest で検査する。

評価と改善のループは `just eval-loop`、保存した故障の再現は `just replay-loop`。[evals/](evals/README.md) に仮説・条件・run・判断・次の仮説を残す。初回の幅広い比較は `just evaluate` / `just replay-evaluation` で再実行できる。実験用の探索器・履歴 checker は public API に含めない。

時間・メモリから worker 数と探索深さを選ぶ `./budget`、独立 task の分割・集約を行う `./sharding` を追加した。[実行時間・Rust・cloud sharding の評価](evals/runtime/README.md) と [測定結果](evals/iterations/0003-runtime.md) を参照。Rust/Wasm は実験用カーネルとして分離している。

独立した検査の配置と常駐workerへの配布は `./scheduling`。[予算・費用順配置・batch化のガイド](docs/scheduling.md) と [TLA検査の1/2/4/8-worker評価](evals/iterations/0017-parallel-safety.md) を参照。共通APIはNodeやsolverに依存せず、Node / browserのTLA adapterは評価用の試作として分離している。

<!-- checker-comparison:start -->
## モデル検査の速度比較

同じTLA+ source/configを各回別processで解析し、完全探索した起動込み時間。単位は秒、各条件1 warmup後3回の中央値。Apple M5 arm64、32 GiB、Node 24.22.2、tla-ts 0.0.10、TLC v1.7.4 / Temurin 21.0.13+9で測った。

| モデル      |    状態数 | tla-ts 1 worker | TLC 1 worker | TLC 8 workers | idealis Wasm 1 worker | idealis native 1 worker | idealis native 8 workers |
| ----------- | --------: | --------------: | -----------: | ------------: | --------------------: | ----------------------: | -----------------------: |
| カウンタ群  | 5,308,416 |           5.190 |        6.134 |         6.476 |                 3.818 |                   2.851 |                    3.646 |
| Boolean関数 | 1,048,576 |           5.388 |       12.647 |         4.444 |                18.666 |                   9.345 |                    5.101 |
| チャネル群  | 2,097,152 |          16.371 |       23.825 |         6.807 |                31.415 |                  20.733 |                    9.537 |
| 幅1のring   | 1,000,000 |           0.410 |        0.900 |         1.440 |                 0.244 |                   0.186 |                    0.187 |

Wasm列はNode上の実行で、ブラウザの速度ではない。nativeの幅1モデルはpoolを起動しない。tla-tsは今回1 workerのみを測っており、その並列性能との比較はしていない。TLCとNodeのheap設定は8 GiBで、共通の厳密なRSS上限ではない。idealisは対応する有限fragmentを検査し、TLA+全体の互換性やモデル全般の速度を保証する結果ではない。

[全sample・最小/最大・条件と再実行](evals/iterations/0022-readme-comparison.md)を保存した。表は同じrunから生成し、過去の値を混在させていない。常駐APIの比較は[別の0016測定](evals/iterations/0016-checker-benchmark.md#apiを再利用した全探索)を参照。上の起動込み時間と同じ指標として扱わない。

<!-- checker-comparison:end -->

## Rustコアの最適化前後

上の6経路比較は最適化前の0022時点の記録。今回の6項目の最適化は、変更前後のnative/Wasmを別途交互に測った。起動込み秒、1 warmup後3回の中央値。比率は変更前/変更後。TLC/tla-tsの過去値はこの表へ混ぜていない。

| モデル      | native 1: 変更前 → 後 (秒) |  比率 | native 最大8: 変更前 → 後 (秒) |  比率 | Wasm 1: 変更前 → 後 (秒) |  比率 |
| ----------- | -------------------------: | ----: | -----------------------------: | ----: | -----------------------: | ----: |
| カウンタ群  |              3.114 → 3.061 | 1.02× |                  3.781 → 2.996 | 1.26× |            3.944 → 4.822 | 0.82× |
| Boolean関数 |              9.609 → 7.221 | 1.33× |                  5.407 → 3.942 | 1.37× |          19.809 → 10.585 | 1.87× |
| チャネル群  |            22.260 → 17.838 | 1.25× |                 13.500 → 8.829 | 1.53× |          38.771 → 24.964 | 1.55× |
| 幅1のring   |              0.213 → 0.215 | 0.99× |                  0.223 → 0.217 | 1.03× |            0.268 → 0.286 | 0.94× |

最大8のcounterは軽いscalar演算と判定して逐次実行する。Wasmのcounter/ringは後退が残る。Boolean関数のnative RSSは約40%、チャネル群は約35%減った（200 ms間隔の観測）。[Criterion・全sample・範囲・採否と再実行](evals/iterations/0023-criterion-safety.md)に記録した。`just criterion-test`でsmoke、`just criterion-save <stage> <新規runパス>`で14本を測る。

## CPU profileを使った追加調整

0023後の版から、scalarのleaf評価と整数keyの関数検索を調整した。以下はprofilerを外した通常Wasmの新しい交互比較で、起動込み中央値。上の表とは別run。

| モデル     |     変更前 → 後 | before/after |
| ---------- | --------------: | -----------: |
| カウンタ群 | 4.476 → 4.136 s |        1.08× |
| 幅1のring  |    272 → 255 ms |        1.07× |

native Criterionのcounterは12.285→9.266 ms、functionは11.071→9.416 msだった。整数位置の変更単独はchannels最大8で後退する回もあり、全ケースの高速化を保証する結果ではない。[profile・14本の段階別測定・16 sampleと再生](evals/iterations/0024-profile-tuning.md)を保存した。`just profile-test`、`just profile-criterion <stage> <新規runパス>`で確認できる。

## モデルと実装の比較

TS / Rust の共通コアは `./portable`。同じ Rust ソースを native と Wasm で使い、適合性照合とトレース再生を行う。[Go adapter](docs/go-adapter.md) は CGO 不要で同じ Wasm に接続する。[値・effect protocol・使用例](docs/portable-core.md) を参照。`just portable-build` / `just portable-test` で Rust と Node・実ブラウザ、`just go-test` で Go の照合と単独 consumer を検査する。

[共通オラクルで3言語の実装を動かす評価](docs/shared-oracle.md) は `just cross-test`。同じ有限モデルと保存 trace を使い、正常版・故障版の TS / Rust / Go を直接操作して比較する。

[非同期履歴とモデル再利用](docs/async-history.md) は `./history` と `loadFiniteModel`。未完了・結果不明の操作を含む照合、索引付き handle、依存関係を使う縮小と探索 feedback の評価は `just async-test` から実行できる。

[TLA+ の Rust backend](docs/rust-safety.md) は `compileTlaSafety` と `checkSafety`。整数・集合・関数・record・sequence の有限モデル、temporal property、WF / SF を IR に変換し、状態生成・BFS・SCC 検査・反例復元を native / Wasm 内で実行する。全 TLA+ 互換ではなく、対応外のモデルは既存の tla-ts を使う。

検査器は独立した[`rtlc` crate](crates/rtlc)へ分離した。IR・値評価・探索・temporal/fairness・native並列化を持ち、idealis-coreへの依存はない。idealis-coreは実装照合・履歴・host・縮小と既存Wasm/JSONL adapterを担当する。`just rtlc-test` / `just rtlc-example`で単独利用を検査し、`just package`で両crateと単独consumerを確認できる。

構文解析は独立した[`rtla` crate](crates/rtla)、ASTからIRへの変換は[`rtla-compiler`](crates/rtla-compiler)が担当する。[Rust frontend](docs/rust-frontend.md)の`compileTlaSource` / `checkTlaSource`で、ブラウザでもtla-tsなしでソースの解析・変換・検査を実行できる。既存の`compileTlaSafety`はtla-tsを使う比較対象として残す。`just compiler-test` / `just compiler-eval <新規run>`で検査し、`just package`で5つのcrateとWasmの配布物を確認できる。

[Apalacheとの比較](docs/apalache-comparison.md)は、SMTによる有界検査とrtlcの状態列挙の保証を分け、同じ6条件の起動込み時間と反例を記録する。小規模pilotを大規模モデルの速度順位へ一般化しない。

[`@mizchi/idealis/backend`](docs/backend-contract.md)は、完全探索、有界の反例なし、帰納性を区別する共通契約。model/property/意味論/仮定/backend版を結果へ結びつけ、空の初期集合・資源切れ・unknownを検査成功へ変換しない。rtlcのnative/Wasm bridgeを提供する。Apalacheの汎用adapterとSMT encodingは今後の対象。[契約と保存応答の検査](evals/iterations/0028-backend-contract.md)に結果を記録した。

```ts
import { checkConformance, checkModel, checkTrace } from "./src/index.js";
import { counterModel, createCounter } from "./examples/durable-counter.js";

const result = await checkConformance(counterModel, createCounter(), [
  { type: "add", id: "request-1", amount: 2 },
  { type: "crash" },
  { type: "restart" },
  { type: "add", id: "request-1", amount: 2 },
]);
if (result.status !== "matched") throw new Error(JSON.stringify(result));

const oracle = checkModel(counterModel, { maxDepth: 4, maxStates: 500 });
for (const trace of oracle.traces) {
  const replay = await checkTrace(trace, createCounter());
  if (replay.status !== "matched") throw new Error(JSON.stringify(replay));
}
```

例は独立した immutable な参照モデルと mutable な実装で構成する。障害は `crash` / `restart`、ネットワークは `send` / `deliver` / `drop` の操作として明示する。配信インデックスを選ぶことで遅延・並べ替えを再現する。OS の実際の通信やディスク障害を注入する機能は含まない。

`Model.initial()` と `Model.step()` は許容する状態と観測の候補を返す。`Driver.reset()` と `Driver.step()` は実装を操作して観測を返す。非決定的なモデルでは、観測に合う候補状態を履歴全体で絞る。ドライバーの例外は呼び出し元に伝わり、`dispose()` は終了時に呼ばれる。

## Property-based testing と差分テスト

```ts
import assert from "node:assert/strict";
import { array, constantFrom } from "fast-check";
import { checkProperty } from "./src/property.js";
import { checkDifferential } from "./src/index.js";
import {
  counterModel,
  createCounter,
  type CounterAction,
} from "./examples/durable-counter.js";

const actions = array(
  constantFrom<CounterAction>(
    { type: "add", id: "a", amount: 1 },
    { type: "crash" },
    { type: "restart" },
  ),
  { maxLength: 24 },
);

const result = await checkProperty(counterModel, createCounter, actions, {
  seed: 42,
  numRuns: 100,
});
assert.equal(result.status, "passed");

const diff = await checkDifferential(createCounter(), createCounter(), [
  { type: "add", id: "a", amount: 1 },
]);
assert.equal(diff.status, "matched");
```

このコードは Vitest の `test()` 内でも `node:test` 内でも動く。`persist: false` の故障実装では、反例が `add → crash → restart` に縮小する。結果の `seed` と `path` を同じ arbitrary・fast-check バージョンとともに渡すと再実行できる。長期保存には操作列自体も残す。無効操作で全件が棄却される場合は `exhausted` になる。

デモは `.artifacts/oracle.json`、`tla-oracle.json`、`counterexample.json` を生成する。`decodeTrace` は action / observation の型ガードを要求する。トレースはバージョン付き JSON で、bigint・Set・Map はアダプターで明示的に符号化する。

## アダプターと移植性

| import            | 役割                                                   | ランタイム                     |
| ----------------- | ------------------------------------------------------ | ------------------------------ |
| `src/index.ts`    | 有限モデルの BFS、合成、適合性比較、差分比較、トレース | 外部依存のない TS              |
| `src/property.ts` | 生成、縮小、seed/path 再現                             | fast-check                     |
| `src/tla.ts`      | 実際の TLA+ 検査結果と有限反例の変換                   | tla-ts                         |
| `src/smt.ts`      | SMT-LIB の SAT/UNSAT/UNKNOWN と整数解の取り出し        | 呼び出し側で初期化した Z3 Wasm |

コアを import してもソルバーは読み込まれない。Chromium の E2E は4つの経路を実行する。Z3 のブラウザ実行には `z3-built.js` / `.wasm` の配信、SharedArrayBuffer、COOP/COEP ヘッダーが必要。

`tla-ts@0.0.10` は Node/Bun 向け参照を含むため、ブラウザで通常の import をそのままバンドルすると失敗する。この試作では esbuild の `alias: { "node:zlib": "fflate" }`、`external: ["bun:jsc"]`、`define: { global: "globalThis" }` を使う。Bun 専用の動的 import はブラウザでは到達しない。TLA+ は既定の単一ワーカーで検査する。具体的な配信とビルドは [ブラウザテスト](tests/browser.spec.ts) を参照。

Z3 は `init()` の結果を `createSmtOracle(api)` に渡す。Node では利用後に `killThreads(api.em)` を呼ぶ。整数解を十進文字列で返し、無制限整数の精度を保つ。SMT-LIB の assertion を渡す API であり、任意の TS 関数を自動で記号実行する機能はない。

## 検証の範囲

- モデル検査の `complete` は、提供された `actions` と衝突のない `key` に対して到達グラフが閉じたことを表す。打ち切りは `bounded`、初期状態なしは `empty`。TS コアの検査対象は状態不変条件で、liveness / fairness は含まない。
- 生成トレースは到達状態・遷移の代表的な witness。実装の全履歴を網羅するテスト集合ではない。モデル側の状態が同じでも、実装には異なる隠れた状態があり得る。
- 適合性比較は指定した操作列・観測・抽象化に対する検査。`invalid` はモデルで実行できない操作、`mismatch` は許容観測からの逸脱。汎用の refinement proof や線形化可能性の証明ではない。
- `composeModels` は独立操作の interleaving と明示した `both` 操作の直積を探索する。モデル間の境界不変条件を指定できる。assume-guarantee 推論は行わない。
- TLA+ アダプターは成功時に witness を捏造しない。lasso の反例は有限オラクルへ変換せず、元の検査結果を残す。型や整数意味論を Z3 と自動で一致させる変換は含まない。

[設計](docs/architecture.md)、[テスト手法の調査](docs/testing-research.md)、[検証記録](docs/validation.md) を参照。
