# Resonara

ScarletUI + SGFXによるLinux / macOS向けネイティブDAWの初期版。既存リポジトリとは独立したRust workspaceです。

## 起動

Nixをインストールした環境で、次を実行します。依存するScarletUIはCargoがGitから取得するため、隣接チェックアウトは不要です。

```sh
git clone https://github.com/petitstrawberry/resonara.git
cd resonara
./scripts/dev cargo run --locked --release -p resonara
```

ScarletUIはGit revision `91238ed12c3379c0aea7e7ba86e73139cfb1f550` に固定しています。SGFXもScarletUIが指定する固定Git revisionを使い、推移的依存の正確な解決結果を`Cargo.lock`に記録しています。依存先のローカル変更やパッチは不要です。

```sh
./scripts/dev cargo run --locked --release -p resonara
# 空のプロジェクト
./scripts/dev cargo run --locked -p resonara -- --empty
# 保存済みプロジェクト
./scripts/dev cargo run --locked -p resonara -- --project session.resonara.json
```

`scripts/dev` はPATH上の `nix` を使います。別のNix実行ファイルやnix-portable用ラッパーを使う場合は `RESONARA_NIX=/path/to/nix-wrapper ./scripts/dev ...` を指定できます。ラッパーは通常の`nix`と同じ引数を受け取ってください。Cargoのキャッシュ・出力先は `CARGO_HOME`・`CARGO_TARGET_DIR` で任意に設定できます。`nix develop` でも開発できます。flakeはboxcraftと共有開発環境を参考に、Scarlet RustとNixpkgsを固定しています。LinuxではALSA、Vulkan/Mesa、X11/Wayland、フォントを用意し、CPALがALSAを使います。macOSではCPALがCoreAudio、ScarletUI/SGFXがネイティブウィンドウとGPUを使います。

ScarletUI自体を編集する場合に限り、未コミットのCargo `[patch]` 設定でローカルチェックアウトへ切り替えられます。通常のビルド・検証は上記のGit依存と`--locked`を使い、マシン固有のpath指定をコミットしないでください。

## 操作

- 起動時は3トラックのデモ。上部のImport WAVからアプリ内ファイルブラウザーで音声を選び、トラックを追加。
- 上部にトランスポート、中央にトラック一覧とアレンジメント、左にインスペクター、下部にミキサー。ウィンドウのリサイズに追従し、インスペクター・ミキサーは開閉可能。
- Spaceで再生・停止、Homeで先頭へ。ルーラーのクリック・ドラッグで再生位置を変更。停止時の位置を保持。ルーラードラッグはEscapeで取り消し、再生中なら離した位置から再開。
- カウンターとルーラーは小節・拍・tickが初期表示（四分音符あたり960 tick）。角丸の同じパネルにBPMと拍子の入力をまとめ、BPM欄に20〜400、METER欄に3/4・6/8などを入力しEnterで確定。分子1〜32、分母1・2・4・8・16・32に対応。BPM・拍子はプロジェクトに保存しUndo/Redo可能。表示ボタンで小節・拍／時間／サンプル位置を切り替える。値のない旧プロジェクトは120 BPM・4/4として読み込む。音声の位置・長さはサンプル単位のまま保持する。BPM・拍子の確定時は再生を停止し、タイムストレッチは行わない。
- トラックヘッダーやリージョンをクリックして選択。リージョン中央をドラッグして移動、端をドラッグして非破壊トリム。Escapeでドラッグを取り消す。Sで再生位置を分割し、Splitツールでは波形をクリックして分割。
- ステレオ素材のリージョンはL/Rの独立した2段波形、モノ素材は1段波形で表示。元WAVのチャンネル数を分割・トリム・保存後も保持。以前のJSONにチャンネル数の記録がない場合は、音声を失わないようステレオとして読み込む。
- Snapは100msグリッド。＋/−でズーム、Fで全体表示。トラック一覧・ミキサーはスクロール可能。Fixed/Followで表示範囲の固定と再生位置への追従を切り替える。Followは右端付近で次の表示範囲へ送り、範囲内では波形を再生成しない。
- インスペクターでトラック名、Gain、Pan、Mute、Soloを編集。ミキサーには縦型フェーダー、Panノブ、ピーク表示があり、Masterを含む変更は再生中にも反映。フェーダーのGain dBとメーターのPeak dBFSには、[Spectrumを参照した別々の非線形目盛](docs/mixer-scale-reference.md)を使う。
- トラックとMasterのメーターは実際の左右信号を別々に表示。Masterはトラックのピーク値を足すのではなく、合成した音声のMaster gain適用後・クリップ前を測定。数値へホバーするとL/Rの詳細を確認でき、ピーク保持・クリップ表示は時間経過と再生開始でリセット。
- トラックヘッダーの右クリック（MacはControlクリックも可）でAdd/Duplicate/Delete。トラック欄の余白ではAdd、左上の＋でも空トラックを追加。インスペクターは選択内容のプロパティを表示。Gain目盛りのクリックで正確な値へ設定可能。リージョン削除、Undo/Redoに対応。Undoは直近100編集で、ミキサー変更も対象。スライダードラッグはひとつの編集として扱う。
- Open/終了時には未保存変更を確認。インポート・読込・保存・書出しはワーカースレッドで処理し、処理中の競合する編集を防止。構造編集やファイル操作は再生を停止する。
- Save projectは音声データを含むJSON形式。元WAVを移動してもOpenできる。Export WAVは48kHz（読込プロジェクトのレート）、ステレオ32bit floatで全体を書き出す。Mute/Solo/Masterを反映し、±1にクリップ。

Ctrl/Cmd＋IでImport、OでOpen、SでSave、EでExport、DでDuplicate、ZでUndo、Shift＋ZでRedo。テキスト入力中は編集ショートカットが誤発火しないように分離しています。その他の操作は右上のHelpから確認できます。

WAVはmono/stereoのPCM 8/16/24/32bitと32bit floatを受け付け、プロジェクトのレートへ線形補間します。分割・トリムは原音声を共有する非破壊編集。保存では共有音声も各クリップへJSONで埋め込むため、大きなセッションには容量・読込メモリ面の制約があります。録音、MIDI、プラグイン、タイムストレッチ、高品質な帯域制限リサンプラー、OS標準のファイル選択ダイアログは未実装です。

## 構成とリアルタイム制約

- `resonara-core`: プロジェクト、非破壊クリップ編集、WAV/保存、ミックスエンジン。ScarletUI、CPAL、ALSAへ依存しない。
- `resonara-platform`: CPALのデバイス・フォーマット選択、ストリーム所有。Scarlet移植時はこのアダプターを置換する。
- `resonara-app`: ScarletUIのUI、SGFX波形描画、キャッシュした波形ピーク、非線形フェーダー・メーター、Panノブ、操作履歴。将来はScarlet用のUIプラットフォームを選択できる構造。

再生開始前にクリップと原音声のスナップショットを作り、ストリーム終了まで保持します。レンダーループにはMutex/RwLock、メモリ確保、所有データの交換・破棄、ファイルI/O、ログ出力はありません。ミキサー、メーター、位置、エラーの受け渡しはアトミック。再生中の構造編集はUIスレッドでストリームを停止してから実行します。CPALやOSドライバー内部の処理まで確保ゼロを保証するものではありません。

音声エンジンは開始前にDAGをコンパイルし、既定128フレームの内部ブロックごとに各ノードを1回だけ処理します。共有ノードの出力を複数の送信先で読み、Masterから再帰的に同じ処理を実行し直しません。バッファとDelayの状態は事前確保し、最終参照後のバッファ領域を再利用します。既存UIはこのスケジューラーでtrack→masterを再生します。

コアの `Engine::with_graph` ではBus、pre/post-fader send、Gain・OnePole・Delay insertを明示したグラフを作れます。循環はDelayを含めて拒否します。Aux編集UI、ルーティングの保存・書出し、エフェクトtail延長、プラグインホスト、PDC、再生中のグラフ差し替えは未実装です。API、信号順序、制限、テストとスケール試験は[音声グラフ](docs/audio-graph.md)に記載しています。

Scarlet版の音声デバイスアダプター・クロスビルドは未実装です。コアは現在stdを使いますが、デスクトップ固有のデバイス・ウィンドウAPIから分離しています。

ファイル選択は現在アプリ内ブラウザーです。今後は共通の`FileDialog`境界を設け、macOS・Linux・将来のScarletで各プラットフォームのファイル選択へ委譲する設計とします。現在のブラウザーはフォールバックとして維持し、OS固有処理は音声コアへ持ち込みません。このプラットフォーム別ダイアログ実装は今回の変更には含まれていません。

## 検証

```sh
./scripts/dev bash scripts/verify
./scripts/dev bash scripts/headless bash scripts/gui-smoke
```

`verify` はGit依存の固定・同一性、フォーマット、workspaceテスト、ビルドを実行します。LinuxではさらにALSAファイル出力の音声スモークを実行します。スモーク用ALSA設定はそのプロセス内だけに適用し、システム設定は変更しません。実デバイスを使わず、CPALのコールバック進行と非ゼロの音声出力を検査します。macOSではALSAスモークを明示的にスキップし、通常起動時のCoreAudio再生を別に確認します。`headless`と`gui-smoke`はLinux専用で、Macではネイティブウィンドウを使います。GUIスモークは毎回新しい出力フォルダーを作り、過去の保存・書出し結果で誤って成功しないようにしています。`RESONARA_BIN`でGUI検証用の実行ファイルを変更できます。

`artifacts/` に検証ログ、保存・再読込プロジェクト、書出しWAV、再生音声、画面PNGを生成します。テストではミックス値、Mute/Solo/Pan/Master、分割後の44.1/96kHz再生一致、保存・書出し、各PCM幅、不正データ、Undo/Redo、キャンセル、非同期I/O、テキスト入力とショートカット、フェーダー、ElementTree上のポインター配送を検証します。スレッド単位の計測allocatorでrenderを1000回実行し、確保・解放ゼロを検査します。

5分4秒の実録音を使った8トラック負荷試験のライセンス、再現方法、測定値は[実音声テスト](docs/audio-fixture.md)を参照してください。音声データと生成物はGit管理対象外です。CPUミックス性能、UIモデル・レイアウト性能、画面上の動作確認、実機オーディオ性能は区別して記録します。

2026-10-01のLinuxクラウド検証では固定flakeの評価に成功しましたが、コンテナーがprivate mount namespaceを許可しないため`nix develop`の起動は未成功です。同じ固定Scarlet Rustを直接起動した環境ではビルド・テスト・ALSA出力に成功しています。制限された環境でのGUI検証には許可された既存デスクトップを使い、セキュリティ設定を変更していません。

Mac移行では既存Nix経由のdebug/releaseビルドと125テストが通過し、Apple M3 ProのMetal GPUで起動を確認しました。ユーザーの操作でCoreAudioの出音も確認済みです。ScrollView操作の重さ、途中起動の汎用`UI: RenderError`は未解決で、実機フレーム間隔・レイテンシ・ドロップアウトは未計測です。[検証記録](docs/validation.md)と[UI計測の限界](docs/ui-performance.md)を参照してください。
