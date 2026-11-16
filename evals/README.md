# 評価しながら改善する

モデル検査・テスト手法の採用判断を記録する場所。仮説、条件、実行、判断、次の仮説を1つの iteration として残す。比較器の `improved` は故障ごとの検出数の増加を表し、費用や外的妥当性を含む採用決定ではない。

[0030: RustのSMTサブセット](iterations/0030-smt-subset.md)は生成入力とSATモデルをZ3に照合する。[0031: Z3の再利用を含む速度比較](iterations/0031-smt-reuse.md)でnative/Wasm、解析、prepared、push/pop、起動の費用を分けた。限定した差分制約の経路は候補とし、Boolean UNSATの後退とunknownも保存している。

[0034: 静的Alloyサブセット](iterations/0034-alloy-subset.md)は、独立crate `ralloy`のnative/Wasmと公式Alloy 6.2.0を比較する。有限scopeの関係モデル、返却構造の受理、空関係のarityの修正、ブラウザと単独consumerを検査した。

[0035: Alloyモデルと固定instance](iterations/0035-alloy-instance.md)は、実装の関係スナップショットをnative/Wasmで評価する。公開TS API、node:test/Vitest、全512グラフ、公式Analyzerへの構造固定、実Chromiumと単独consumerを検査した。

[0036: Rust testing core](iterations/0036-testing-core.md)は、生成・縮小・差分比較・保存caseの再生を独立crateへ移した。同じcorpusでnative/Wasm、TS生成器と既存縮小器を照合し、状態付きカウンタとブラウザ・配布consumerを検査した。

[0037: SAT/CNF候補](iterations/0037-sat-foundation.md)は、Varisatのnative/Wasmを独立全代入列挙へ照合した。実Chromiumでも動く候補として残し、solver内部の実行バジェットを保証するbackendへの採用は保留している。

```sh
just eval-loop
just replay-loop
```

`eval-loop` は既定の plan で開発用・確認用 seed を実行し、新しい run ID を作る。`replay-loop` は台帳で最新の完了した確認 run を選び、故障版で失敗・正常版で通過することを検査する。

plan と run ID を明示する場合:

```sh
pnpm eval:loop evals/plans/0001-longer-walk.json my-run
pnpm eval:replay evals/runs/my-run-confirmation
```

| 場所                       | 記録するもの                                                       |
| -------------------------- | ------------------------------------------------------------------ |
| `methods.md`               | 手法の定義、比較単位、前提、解釈の制約                             |
| `plans/`                   | 結果を見る前に固定する仮説、変更点、予算、seed、判断の観点         |
| `runs/<id>/manifest.json`  | plan、ソースの SHA-256、revision、dirty paths、実行環境、親 run    |
| `runs/<id>/trials.json`    | 正常版と全故障の seed ごとの検出・見逃し・コスト・再現結果         |
| `runs/<id>/corpora.json`   | 生成ケース数、状態・遷移、生成時間、要求操作数                     |
| `runs/<id>/witnesses.json` | 故障版を元のモデルで検査し直す操作列                               |
| `runs/<id>/result.json`    | 集計、正常対照、評価、各証拠の digest。完了の目印                  |
| `runs/ledger.jsonl`        | run の追記台帳。例外は `error`、正常対照の失敗は `controls-failed` |
| `iterations/`              | 結果の解釈、採用・保留の理由、後退、次の仮説                       |
| `harness/`                 | 実験用のモデル・実装・探索器。public API と分離                    |

## ループの進め方

1. 前の iteration の見逃し・費用・調査の難しさから、変更点を1つ選ぶ。
2. plan を作り、結果を見る前にコミットする。モデル、正常版、故障集合、予算、開発用・確認用 seed を明示する。
3. 実装とテストを整え、開発実行を記録する。
4. 同じ plan / ソースのまま確認用 seed で実行する。途中の変更、重複 seed、変更された証拠を拒否する。
5. 数値だけでなく witness を再現する。正常版の誤検出、無効操作、再現失敗は判断の前提を崩す。
6. iteration に判断を追記する。後退がある場合も保存し、次の仮説へ進む。

既存 run ID は上書きできない。中断や例外で `result.json` がない run を合格として扱わない。結果・台帳を編集して再試行を隠さず、新しい ID で実行する。formatter の対象から `runs/` を除外し、元の証拠を保つ。

確認用 seed も同じ人工故障集合を使う。これはスケジュールに対する確認であり、未知故障や実システムへの一般化を検証したことにはならない。確認結果を見て次の仮説を作る場合は、別の確認用 seed を予約する。

実行済みの比較は [0001: 履歴長16](iterations/0001-longer-walk.md)、[0002: 8/16の混在](iterations/0002-mixed-depth.md)。次の仮説は [backlog](backlog.md) に残す。CI で同じ plan を反復するのは再現・回帰確認であり、新しい独立した確認実験として集計しない。

初回の幅広い pilot は `pnpm evaluate` / `pnpm evaluate:replay` で再実行できる。履歴は [0000](iterations/0000-pilot.md)。古い記録には後付けの厳密な provenance を主張せず、`legacy-import` として区別した。

実行時間と Rust 化は [0003](iterations/0003-runtime.md)。[runtime/](runtime/README.md) に budget planner の前提、Rust/Wasm の比較、クラウド分割 CLI を記録している。

[0007 の計画](plans/0007-shared-oracle.md) は、同じ仕様 oracle で TS / Rust / Go の実装を直接操作する。`just cross-test`、`cross-eval` / `cross-replay` と [利用手順](../docs/shared-oracle.md) を参照。

[0008 の結果](iterations/0008-async-tuning.md) は、未確定操作を含む非同期履歴、障害境界、model handle、trace buffer、依存縮小、feedback 探索の採用判断。`just async-test`、`async-eval` / `async-replay` と [履歴の意味・API](../docs/async-history.md) を参照。

[0009〜0011 の結果](iterations/0009-rust-safety.md) は、TLA+ safety fragment の状態生成・BFS を Rust native / Wasm 内で行う backend と、literal decode・状態保持の改善。TLC32件、変更前後37入力、各段階の固定入力測定を保存した。`just safety-test` と `safety-eval` / `safety-replay`、[対応範囲](../docs/rust-safety.md) を参照。

[0012 の計画](plans/0012-collections-temporal.md) は、有限集合・関数・record・sequence、有限量化、temporal property と WF / SF の Rust backend 対応。`just complex-eval` / `complex-replay` は入力・結果・反例・対照を保存して再現する。`complex-tlc` / `complex-tlc-replay` は同じ入力と強制した反例を TLC で検査する。`safety-eval` は以前の6 scalar 入力と4つの複合値・temporal 入力を測る。

[0012〜0014 の結果](iterations/0012-collections-temporal.md) に、複合値・temporal の対応、49入力・TLC46入力・反例14件、整数比較と intern table の改善、一律の高速化には至らない条件を記録した。

[TLC直接比較の結果](iterations/0015-tlc-differential.md) は、境界値と seed 固定の小さい有限モデルを TLC / Rust native / Wasm へ渡す差分検査。`just tlc-diff` / `tlc-diff-replay` で判定・計数・返却値・反例を確認する。TLCを拒否対照、モデル評価エラー、実行基盤の失敗と分け、JVMごとに標準モジュールの展開先を隔離する。

[速度比較の結果](iterations/0016-checker-benchmark.md) は、同じ11モデルを tla-ts / TLC / idealis native / Wasm で完全探索した測定。`checker-bench` / `checker-bench-replay` で起動込みとAPI再利用時を分け、source解析を含む時間・状態数・生成数・生ログを保存する。

[常駐poolの評価](iterations/0017-parallel-safety.md) は、独立したTLA検査を1/2/4/8 workerへ配る。`parallel-safety-test` / `parallel-safety-eval` / `parallel-safety-replay` で、予算によるworker数の選択、費用順配置、batch化、結果と反例の一致を確認する。1モデル内部のBFS並列化とは区別する。

[TLCとのbatch比較](iterations/0018-tlc-throughput.md) は、同じ88件を指定探索worker合計8で実行する。TLCの1 JVM × 8 worker / 8 JVM × 各1 workerとidealis native / Wasmを比較し、JVM起動とIR再利用を含むthroughputの差を記録する。`tlc-throughput` / `tlc-throughput-replay` で実行できる。

[単一モデルの比較](iterations/0019-single-model-scaling.md) は、約2,100万遷移の幅広い3モデルと幅1の対照を使い、TLC 1/2/4/8 workerと逐次native/Wasmを測る。`single-model-test` / `single-model-eval` / `single-model-replay`で正しさ・全体時間・補助区間・RSSを保存する。TLC内部方式の移植検討は[並列探索の調査](../docs/tlc-parallel-adoption.md)を参照。

[nativeの層同期BFS](iterations/0020-frontier-safety.md)は、単一モデルを並列展開する明示的な経路。共有workカウンタの競合による後退と、初回の幅1測定への干渉も記録する。`frontier-safety-test` / `frontier-safety-eval` / `frontier-safety-replay`で検査する。

[workerごとの予算予約](iterations/0021-worker-budget.md)は、共通counterへの毎tickの競合を減らす。同APIの1→8 workerで関数・チャネルは中央値約1.8倍、counterは遅くなるため、明示的な経路として採用する。`frontier-budget-eval` / `frontier-budget-replay`で固定入力の測定・再生を行う。

[READMEの6経路比較](iterations/0022-readme-comparison.md)は、同じ4入力をtla-ts 1、TLC 1/8、Wasm 1、native 1/8で測り直した。条件・全sample・再生とREADMEへの照合を保存する。

[Criterionによる最適化](iterations/0023-criterion-safety.md)は、値の一重保持、EXCEPT、型所属cache、候補buffer、重複検査、worker/batch選択の6項目。`criterion-test` / `criterion-save`でnative kernel/APIを測り、`criterion-compare` / `criterion-replay`で固定binaryの前後・起動・RSS・正常/故障を確認する。Wasm scalarの後退も記録した。

[CPU profileとmicro tuning](iterations/0024-profile-tuning.md)は、Wasm/nativeのraw profileとself/inclusive集計からleaf評価と整数位置検索を調整した。`profile-record` / `profile-summary`でprofile、`profile-criterion`で14本、`profile-scalar` / `profile-replay`で通常Wasmの交互比較と再生を行う。

[rtlcへの分離](iterations/0025-rtlc-extraction.md)は、有限TLA+ fragmentの検査をidealisから独立したcrateへ移した。`rtlc-test` / `rtlc-example`と単独package consumer、分離前後のnative964件・Wasm241件で既存挙動を確認した。探索アルゴリズムと速度の主張は追加していない。

[rtlaのparser](iterations/0026-rtla-parser.md)は、TLA+／TLC設定を型付きASTへ解析する独立Rust crate。`rtla-test` / `rtla-browser`でnative/Wasm、`rtla-eval`で199組のASTとSANYの構文対照を検査する。IR生成はTS側に残し、対応fragmentと全TLA+を区別する。

[Apalacheとの比較](iterations/0027-apalache-comparison.md)は、BMC/帰納性と有限グラフのcompleteを区別する。`apalache-test` / `apalache-eval`で同じsource/configのpilotを実行し、raw log、ITF、IR、反例検証と起動込み時間を保存する。

[backend契約](iterations/0028-backend-contract.md)は、explicit complete、有界の反例なし、帰納性と個別obligationを分ける。`backend-test` / `backend-eval`で実rtlc native/Wasm、0027の保存したApalache応答、非空性と帰納性の集約条件を確認する。新しい速度比較や汎用Apalache adapterの評価ではない。

[Rust frontend](iterations/0029-rtla-compiler.md)は、rtlaのASTからrtlcのIRを生成する。`compiler-test` / `compiler-eval`で既存TS frontendとのIR・結果の一致、`compiler-tlc`で固定20件の直接比較を行う。意味の一致を確認する回帰で、性能や未知故障発見の実験とは区別する。
