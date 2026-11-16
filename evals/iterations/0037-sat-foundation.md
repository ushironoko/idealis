# 0037: SAT/CNF候補の可搬性と停止契約

Varisat 0.2.2を使う独立したeval crateで、nativeとホストimportのないWasmによるCNFのsolveを確認した。増分追加とassumptionsを独立したTS全代入列挙へ照合した。CNF候補として残すが、実行バジェットを保証するbackendへの採用は保留する。[事前計画](../plans/0037-sat-foundation.md)、[実行前の具体化](../plans/0037-sat-protocol.md)、[再実行手順](../sat-foundation/README.md)を参照。

## 判定と具体値の照合

| run                                                                   | 入力数 | solve round | SAT | UNSAT | エラー・予算未対応の対照 |
| --------------------------------------------------------------------- | -----: | ----------: | --: | ----: | -----------------------: |
| [開発](../runs/0037-sat-development/result.json)                      |     64 |         183 |  66 |   117 |                        6 |
| [確認](../runs/0037-sat-confirmation/result.json)                     |    128 |         375 | 137 |   238 |                        6 |
| [Chromium回帰](../runs/0037-sat-verification/browser-records.json.gz) |     32 |          87 |  39 |    48 |                        1 |

開発・確認の計558 roundでnative/Wasmと全代入列挙の判定が一致した。SATの両モデルはそれぞれ元の式へ代入して確認し、同じ代入であることは要求していない。全solveでVarisatの自己検査を有効にした。空式・空節、重複literal、tautology、矛盾assumptionsからの回復と永続する節追加を含む。

sourceVersionは`c4de8f9b0dc5ca6044b5a876e9e52b36db562971d50b46b22b12bbfeb7bb476c`で共通。保存した入力・応答は同じソース・binaryで全件再生した。各runには7件の固定入力を含む。192件すべてが別の入力という意味ではない。seedは開発0x37a1、確認0x37c3、ブラウザ回帰0x3701。ブラウザ回帰を独立した確認実験に数えない。

ブラウザの87 roundはnativeと全列挙に一致した。Wasmは282,868 bytes、importは0件で、cross-origin isolationなしの実Chromiumでsolveした。同じartifactのnative binaryは484,800 bytes。これはJSON adapterと自己検査を含む評価用binaryのサイズで、公開配布物ではない。

## 停止能力の調査

固定crateの`solver.rs`、`config.rs`、`schedule.rs`を調べた。[保存したAPI一覧とhash](../runs/0037-sat-verification/source-review.json)から、[該当revisionのSolver](https://github.com/jix/varisat/blob/851f1747fa5cf83ad01eaf17cb00f45376a7343b/varisat/src/solver.rs)を参照できる。Cargoに入ったsolverのソースは同revisionの公式ソースとSHA-256が一致した。

`Solver::solve`は非公開のschedule loopを回す。公開メソッドと設定には、直接のwork/conflict上限や呼び出し側の中断・再開操作を確認できていない。`SolverError::Interrupted`はrecoverableだが、このエラーだけで停止要求の入口が得られるわけではない。proof processorを使う停止adapterは今回実装していない。

試作は`maxWork: 0`と`maxWork: 100`の両方をsolve前に`unsupported/work-budget`へ分けた。不正literal、変数上限超過、roundなしはerrorになる。例外や未対応をUNSATへ変換しない。ブラウザWorkerの破棄をsolver内部の予算や再開能力とは扱わない。

## 検証と採用判断

Rustの3 tests、node:testの3 tests、実Chromiumを実行した。照合器が誤ったUNSATと不正なSATモデルを拒否する対照も含む。`just check`、`just browser`、`just runtime-build`、`just runtime-test`、`just evaluate`、`just replay-evaluation`のログは[検証run](../runs/0037-sat-verification/result.json)へ保存した。公開coreや配布物は変更していない。

全validationの観測時間は開発約177 ms、確認約192 msだった。起動、IPC、全列挙、model検査を含み、solver速度の比較ではない。自己検査を無効にした場合の費用や、大きいCNFの性能は測っていない。人工的な小規模CNFの一致であり、未知の実プロジェクトの不具合発見もない。

採用するのは隔離した候補と回帰ハーネス。`ralloy`の列挙器や`idealis-smt`は置き換えない。予算対応の候補比較、または停止adapterの実装と再開契約は別計画で評価する。ロードマップの次は明示した並行スケジュールの生成・再生とnative参照への照合を扱う。
