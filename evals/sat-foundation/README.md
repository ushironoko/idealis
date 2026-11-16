# SAT/CNF候補の評価

Varisat 0.2.2を使う非公開のeval crate。有限の連言標準形（CNF）、assumptions、節の増分追加をnative/Wasmで検査する。既存の`ralloy`や`idealis-smt`へは組み込まず、npm/Cargoの配布対象にも追加しない。

```sh
just sat-test
just sat-browser
just sat-replay evals/runs/0037-sat-development
just sat-replay evals/runs/0037-sat-confirmation
```

Rustと`wasm32-unknown-unknown`、Node.js 24+、pnpm、Chromiumが必要。`sat-test`はfmt/clippy、Rustの3 testsとnode:testの3 testsを実行する。`sat-browser`は実Chromiumの32入力をnativeと全代入列挙へ照合する。ブラウザ応答は`.artifacts/sat-foundation/browser/`へ保存する。

## 入力と停止の契約

JSONLの入力は`variables`、`clauses`、`rounds`。literalは1から始まる変数番号で、負数は否定。各roundは`add`と`assumptions`を持つ。追加した節は残り、assumptionsは毎round置き換わる。空式はSAT、空節を含む式はUNSATになる。

評価用の上限は0-8変数、1-16 round、合計256節、JSON入力1 MiB。SATの返却モデルは宣言した変数数のBoolean配列。未割り当ての変数をfalseで補い、元の式とassumptionsを満たすことをRustと独立TSで検査する。UNSATはTSの全代入列挙と照合する。solverの自己検査も全solveで有効にする。

`maxWork`を指定した要求はsolve前に`unsupported/work-budget`を返す。公開Solver APIと設定から、直接のwork/conflict上限や中断・再開の操作を確認できていない。`Interrupted`エラーの存在を呼び出し側の停止能力とは扱わない。proof processorを使う停止adapterは未実装・未評価。入力上限やWorkerの破棄はsolver内部のwork予算ではない。

## 保存と再生

[事前計画](../plans/0037-sat-foundation.md)と[corpusの規則](../plans/0037-sat-protocol.md)を先にコミットした。開発64入力はseed 0x37a1、確認128入力はseed 0x37c3。両方に7つの固定入力を含む。ブラウザ回帰は別のseed 0x3701で32入力を使う。

`just sat-eval development <新規runディレクトリ>`で開発を記録し、`just sat-eval confirmation <新規runディレクトリ> <開発runディレクトリ>`で同じソースの確認を記録する。既存ディレクトリは拒否する。予約したseedの反復は再現検査であり、新しい独立実験とは数えない。ソースとlock、binary hash、全入力、native/Wasmの応答と期待判定を保存する。`sat-replay`は同じソース・binaryの全応答を検査する。異なる有効なSATモデルをnative/Wasmの不一致とは扱わない。

[0037の結果と採否](../iterations/0037-sat-foundation.md)を参照。CNF候補として残し、実行バジェットを保証するbackendへの採用は保留する。小さいCNFの一致はAlloyのSAT符号化、SMT理論、分散実装の適合性や一般的な速度の検証ではない。
