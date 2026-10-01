# Resonara

ScarletUI + SGFXによるLinux / macOS向けネイティブDAWの初期版。既存リポジトリとは独立したRust workspaceです。

## 起動

隣接ディレクトリに `scarlet-ui` のチェックアウトが必要です。新しい環境では同じ親ディレクトリで以下を実行します。

```sh
git clone https://github.com/petitstrawberry/resonara.git
git clone https://github.com/petitstrawberry/scarlet-ui.git
git -C scarlet-ui checkout 91238ed12c3379c0aea7e7ba86e73139cfb1f550
cd resonara
nix develop
cargo run --locked -p resonara
```

この環境では `/workspace/scarlet-ui` をそのまま参照します。SGFXはScarletUIが指定する固定git revisionを使います。参照先へのパッチはありません。

```sh
cd /workspace/resonara
./scripts/dev cargo run --locked -p resonara
# 空のプロジェクト
./scripts/dev cargo run --locked -p resonara -- --empty
# 保存済みプロジェクト
./scripts/dev cargo run --locked -p resonara -- --project session.resonara.json
```

`scripts/dev` はNixを使います。このクラウド環境では既存のnix-portableを利用し、他の環境では通常の `nix` を使います。`nix develop` でも開発できます。flakeはboxcraftと共有開発環境を参考に、Scarlet RustとNixpkgsを固定しています。LinuxではALSA、Vulkan/Mesa、X11/Wayland、フォントを用意し、CPALがALSAを使います。macOSではCPALがCoreAudio、ScarletUI/SGFXがネイティブウィンドウとGPUを使います。

## 操作

- 起動時は3トラックのデモ。WAV pathにローカルファイルのパスを入力し、Import WAVでトラックを追加。
- PlayはCursor/start秒から再生、Stopは停止位置をCursorへ反映、Rewindは先頭へ戻す。
- Previous/Next trackで選択。Gain、Pan、Mute、Solo、Masterは再生中にも反映。各トラックのピークを表示。
- Cursor/startでSplit。Cursor/startとEndの範囲をTrim to rangeで保持。Move to cursorは選択トラック全体を移動し、クリップ間の間隔を維持。
- Duplicate/Delete track、編集のUndo/Redo。構造編集とOpenは再生を停止する。Undoは直近32編集。ミキサー変更はUndoの対象外。
- Save projectは音声データを含むJSON形式。元WAVを移動してもOpenできる。Export WAVは48kHz（読込プロジェクトのレート）、ステレオ32bit floatで全体を書き出す。Mute/Solo/Masterを反映し、±1にクリップ。

WAVはmono/stereoのPCM 8/16/24/32bitと32bit floatを受け付け、プロジェクトのレートへ線形補間します。分割・トリムは原音声を共有する非破壊編集。保存では共有音声もJSONへ埋め込むため、大きなセッションには容量面の制約があります。初期版は録音、プラグイン、タイムストレッチ、波形上のドラッグ操作、ファイル選択ダイアログ、高品質な帯域制限リサンプラーを含みません。

## 構成とリアルタイム制約

- `resonara-core`: プロジェクト、非破壊クリップ編集、WAV/保存、ミックスエンジン。ScarletUI、CPAL、ALSAへ依存しない。
- `resonara-platform`: CPALのデバイス・フォーマット選択、ストリーム所有。Scarlet移植時はこのアダプターを置換する。
- `resonara-app`: ScarletUIのUI、SGFX波形描画、操作履歴。将来はScarlet用のUIプラットフォームを選択できる構造。

再生開始前にクリップと原音声のスナップショットを作り、ストリーム終了まで保持します。レンダーループにはMutex/RwLock、メモリ確保、所有データの交換・破棄、ファイルI/O、ログ出力はありません。ミキサー、メーター、位置、エラーの受け渡しはアトミック。再生中の構造編集はUIスレッドでストリームを停止してから実行します。CPALやOSドライバー内部の処理まで確保ゼロを保証するものではありません。

Scarlet版の音声デバイスアダプター・クロスビルドは未実装です。コアは現在stdを使いますが、デスクトップ固有のデバイス・ウィンドウAPIから分離しています。

## 検証

```sh
./scripts/dev bash scripts/verify
./scripts/dev bash scripts/headless bash scripts/gui-smoke
```

`verify` はフォーマット、workspaceテスト、ビルド、ALSAファイル出力の音声スモークを実行します。スモーク用ALSA設定はそのプロセス内だけに適用し、システム設定は変更しません。実デバイスを使わず、CPALのコールバック進行と非ゼロの音声出力を検査します。

`artifacts/` に検証ログ、保存・再読込プロジェクト、書出しWAV、再生音声、画面PNGを生成します。テストではミックス値、Mute/Solo/Pan/Master、クリップの分割・トリム、サンプルレート、保存・書出しの一致、各PCM幅の読込、不正データ、UI操作モデルを検証します。スレッド単位の計測allocatorでrenderを1000回実行し、確保・解放ゼロを検査します。

macOSでの実機ビルド・再生、実音声ハードウェアのレイテンシ・音質はこのLinuxクラウド環境では未検証です。
