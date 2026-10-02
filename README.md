# Resonara

ScarletUI + SGFXによるLinux / macOS / Scarlet向けネイティブDAWの初期版。既存リポジトリとは独立したRust workspaceです。

## 起動

Nixをインストールした環境で、次を実行します。依存するScarletUIはCargoがGitから取得するため、隣接チェックアウトは不要です。

```sh
git clone https://github.com/petitstrawberry/resonara.git
cd resonara
./scripts/dev cargo run --locked --release -p resonara
```

ScarletUIはGit revision `2e5e96a5c29086c3b555c85f7835e81ecf529290` に固定しています。SGFXもScarletUIが指定する固定Git revisionを使い、推移的依存の正確な解決結果を`Cargo.lock`に記録しています。SWS protocol の依存更新はリポジトリ内の portable vendor patch に含めています。隣接 checkout の変更は不要です。

```sh
# 空の新規プロジェクト（Master 0 dB）
./scripts/dev cargo run --locked --release -p resonara
# 保存済みプロジェクト
./scripts/dev cargo run --locked -p resonara -- --project session.resonara.json
```

`scripts/dev` はPATH上の `nix` を使います。別のNix実行ファイルやnix-portable用ラッパーを使う場合は `RESONARA_NIX=/path/to/nix-wrapper ./scripts/dev ...` を指定できます。ラッパーは通常の`nix`と同じ引数を受け取ってください。Cargoのキャッシュ・出力先は `CARGO_HOME`・`CARGO_TARGET_DIR` で任意に設定できます。`nix develop` でも開発できます。flakeはboxcraftと共有開発環境を参考に、Scarlet RustとNixpkgsを固定しています。LinuxではALSA、Vulkan/Mesa、X11/Wayland、フォントを用意し、CPALがALSAを使います。macOSではCPALがCoreAudio、ScarletUI/SGFXがネイティブウィンドウとGPUを使います。

ScarletUI自体を編集する場合に限り、未コミットのCargo `[patch]` 設定でローカルチェックアウトへ切り替えられます。通常のビルド・検証は上記のGit依存と`--locked`を使い、マシン固有のpath指定をコミットしないでください。

## 操作

- 引数なしでは空のプロジェクト（Master 0 dB）で起動。上部のImport audioからファイルを選び、トラックを追加。macOSではOS標準dialogを使用し、未対応環境ではアプリ内ブラウザーへフォールバック。
- 上部にトランスポート、左全高にインスペクター、右上にトラック一覧とアレンジメント、右下にミキサー。インスペクターは重複しない名前入力、insert/send、選択チャンネルの固定フェーダーを表示し、ミキサーが左側へ入り込まない構造です。ウィンドウのリサイズに追従し、インスペクター・ミキサーは開閉可能。
- Spaceで再生・停止、Homeで先頭へ。ルーラーのクリック・ドラッグで再生位置を変更。停止時の位置を保持。ルーラードラッグはEscapeで取り消し、再生中なら離した位置から再開。音符ボタンまたはCでメトロノームを切り替え、小節頭を強調。BPM・拍子と再生位置に同期し、空のプロジェクトでも使用可能。クリック音は書き出しに含めない。
- カウンターとルーラーは小節・拍・tickが初期表示（四分音符あたり960 tick）。角丸の同じパネルにBPMと拍子の入力をまとめ、BPM欄に20〜400、METER欄に3/4・6/8などを入力しEnterで確定。分子1〜32、分母1・2・4・8・16・32に対応。BPM・拍子はプロジェクトに保存しUndo/Redo可能。表示ボタンで小節・拍／時間／サンプル位置を切り替える。小節表示のルーラーはズームに応じて1/4・1/8・1/16音符の目盛りを追加し、小節・四分・八分・十六分で線の長さを変える。値のない旧プロジェクトは120 BPM・4/4として読み込む。音声の位置・長さはサンプル単位のまま保持する。BPM・拍子の確定時は再生を停止し、タイムストレッチは行わない。
- トラックヘッダーやリージョンをクリックして選択。リージョン中央をドラッグして移動、端をドラッグして非破壊トリム。Escapeでドラッグを取り消す。Sで再生位置を分割し、Splitツールでは波形をクリックして分割。
- ステレオ素材のリージョンはL/Rの独立した2段波形、モノ素材は1段波形で表示。元音声のチャンネル数を分割・トリム・保存後も保持。以前のJSONにチャンネル数の記録がない場合は、音声を失わないようステレオとして読み込む。
- Snapは表示モードに対応。小節表示ではズームに応じた音符／小節グリッドを使い、奇数拍子でも小節頭を基準にする。時間表示は100ms、サンプル表示は1サンプル。ルーラー・分割・移動・トリムに適用。＋/−でズーム、Fで全体表示。トラック一覧・ミキサーはスクロール可能。Fixed/Followで表示範囲の固定と再生位置への追従を切り替える。Followは右端付近で次の表示範囲へ送り、範囲内では波形を再生成しない。
- インスペクターの名前入力と選択チャンネルの縦型フェーダー、Pan、Mute、Soloを編集。ミキサーには縦型フェーダー、Panノブ、ピーク表示があり、Masterを含む変更は再生中にも反映。フェーダーのGain dBとメーターのPeak dBFSには、[Spectrumを参照した別々の非線形目盛](docs/mixer-scale-reference.md)を使う。
- トラックとMasterのメーターは実際の左右信号を別々に表示。Masterはトラックのピーク値を足すのではなく、合成した音声のMaster gain適用後・クリップ前を測定。数値へホバーするとL/Rの詳細を確認でき、ピーク保持・クリップ表示は時間経過と再生開始でリセット。
- トラックヘッダーの右クリック（MacはControlクリックも可）でAdd/Duplicate/Delete。トラック欄の余白ではAdd、左上の＋でも空トラックを追加。インスペクターは選択内容のプロパティを表示。Gain目盛りのクリックで正確な値へ設定可能。リージョン削除、Undo/Redoに対応。Undoは直近100編集で、ミキサー変更も対象。スライダードラッグはひとつの編集として扱う。
- Open/終了時には未保存変更を確認。インポート・読込・保存・書出しはワーカースレッドで処理し、処理中の競合する編集を防止。構造編集やファイル操作は再生を停止する。
- Save projectは音声データを含むJSON形式。元音声ファイルを移動してもOpenできる。JSONの読み書きは256KiBのバッファを使用し、保存はflush・同期後に一時ファイルを置換する。Export WAVは48kHz（読込プロジェクトのレート）、ステレオ32bit floatで全体を書き出す。Mute/Solo/Masterを反映し、±1にクリップ。

Ctrl/Cmd＋IでImport、OでOpen、SでSave、EでExport、DでDuplicate、ZでUndo、Shift＋ZでRedo。テキスト入力中は編集ショートカットが誤発火しないように分離しています。その他の操作は右上のHelpから確認できます。

mono/stereoのWAV（PCM 8/16/24/32bit・32bit float）、MP3、FLAC、AIFF、OGG/Vorbis、M4A/MP4（AAC-LC・ALAC）、AAC、CAFを読み込み、プロジェクトのレートへ線形補間します。WAVは従来のパディング許容読込を維持し、その他はSymphonia 0.6.1でデコードします。分割・トリムは原音声を共有する非破壊編集。保存では共有音声も各クリップへJSONで埋め込むため、大きなセッションには容量・読込メモリ面の制約があります。録音、MIDI、任意の外部プラグイン、タイムストレッチ、高品質な帯域制限リサンプラーは未実装です。

## 構成とリアルタイム制約

- `resonara-core`: プロジェクト、非破壊クリップ編集、音声読込・WAV書出し・保存、ミックスエンジン・メトロノーム。ScarletUI、CPAL、ALSAへ依存しない。
- `resonara-platform`: Linux/macOSではCPAL、ScarletではSASのストリーム所有・PCMリングへの送信。
- `resonara-app`: ScarletUIのUI、SGFX波形描画、キャッシュした波形ピーク、非線形フェーダー・メーター、Panノブ、操作履歴。将来はScarlet用のUIプラットフォームを選択できる構造。

再生開始前にクリップと原音声のスナップショットを作り、ストリーム終了まで保持します。レンダーループにはMutex/RwLock、メモリ確保、所有データの交換・破棄、ファイルI/O、ログ出力はありません。ミキサー、メーター、位置、エラーの受け渡しはアトミック。再生中の構造編集はUIスレッドでストリームを停止してから実行します。CPALやOSドライバー内部の処理まで確保ゼロを保証するものではありません。

音声エンジンは開始前にDAGをコンパイルし、既定128フレームの内部ブロックごとに各ノードを1回だけ処理します。共有ノードの出力を複数の送信先で読み、Masterから再帰的に同じ処理を実行し直しません。バッファとDelayの状態は事前確保し、最終参照後のバッファ領域を再利用します。UIはこのスケジューラーで保存済みのtrack／insert／send／bus→masterを再生します。

トラック／Auxごとにインスペクターから出力先、直列insert、sendを編集できます。Logicの流れに合わせ、Busを接続経路、Auxを受け側チャンネルとして表示します。OUTPUT／SENDSで「New Bus → Aux」を選ぶと経路と受け側を同時に作成・接続し、既存の宛先は「Bus N → チャンネル名」で表示します。ミキサーの「+ Aux」から手動作成も可能です。AuxのINPUTには受信Bus番号を表示します。同じAuxをmain output接続でsubgroup、send接続でeffect returnとして使え、両方を同時に受け取れます。既存の縦型フェーダー、Pan、左右メーターは共通です。

- OUTPUTでStereo Outまたは別のBusを選択。INSERTSの空スロットから内蔵DSP、Resonara Gain、Resonara Freeverb、またはインストール済みCLAPを選択。専用のコンパクトなスロットには名前・電源・メニューを表示し、名前クリックでエディター、電源でbypass、右クリック／メニューから順序変更と削除を行います。Enterで編集、Spaceでbypass、Shift+F10でメニューを開けます（スロットへフォーカス後）。エディターはセッション上のポップアップです。Delayは100% wetのdevice-rate sample delayです。
- SENDSは空スロットをクリックして宛先を選択。各sendを1行にまとめ、宛先・dB値・小さなgainノブ・メニューだけを表示します。Pre/Post Pan、bypass、削除、正確なレベル入力はメニューで設定。send量はdB値を併記し、再生中にatomic更新、ドラッグ1回をUndo1回として扱います。メニューのShow receiverでreturnのインスペクターへ移動します。Preはinsert後・fader前、Post Panはfader/pan後。Auxにもinsert/send/outputを設定できます。
- Bus削除では依存するmain outputをStereo Outへ戻し、そのBus宛のsendを削除。Undo/Redoで全ルーティングと選択を復元します。フィードバック循環はDelay、disabled sendを含め拒否し、失敗した変更は履歴や再生状態を変えません。
- ルーティング・insert・send宛先／pre-post切替は再生を停止して適用し、次の再生開始時に一度だけgraphをコンパイルします。通常のgain/pan/muteとsend gainは再生中にatomic更新し、再コンパイルしません。保存・再読込・WAV書出しは同じsession routingを使用し、旧JSONは従来のtrack→masterとして読み込みます。

コアの `Engine::with_graph` による明示的なDAGも使用可能です。CLAPは汎用パラメーターとopaque stateを保存し、インストール済みエフェクトも読み込みます。標準リバーブ [Resonara Freeverb](plugins/resonara-freeverb/README.md) はmacOSとScarletで共通のScarletUI専用画面を使い、5つのノブ・数値入力・プリセット選択を備えます。CLAPの埋め込みGUIから即時反映し、Undoや再生中の変更にも対応します。macOS/Linuxの通常ビルドは実行ファイルの隣の `plugins/` にDSPを生成するため、実行ファイルを移動する際はこのフォルダーも同梱してください。Scarletのimage builderは `/usr/lib/clap/` に配置します。Scarlet側の通常image recipeには[Resonara bundle](platforms/scarlet/README.md)を含めて、本体とプラグインを一緒に配置してください。MIDI、PDC、エフェクトtail延長は未対応です。制限・ビルド・検証は[CLAPホスト](docs/clap-host.md)を参照してください。API、信号順序、制限、テストとスケール試験は[音声グラフ](docs/audio-graph.md)に記載しています。

Scarlet版はネイティブstdターゲットでSWS UIとSAS音声アダプターを選択します。Git依存・既存vendorパッチを保持し、ネイティブstdでファイルI/Oを共有します。ビルドとQEMU実行の手順、音声の制約、検証段階は[Scarlet移植](docs/scarlet-port.md)を参照してください。クロスビルド成功と実機動作は区別します。

Open／Save As／Audio Import／ExportはScarletUIの共通`file_dialog::FileDialog` APIを使用します。macOSではowner windowに紐付けたNSOpenPanel／NSSavePanelを非同期で表示し、初期フォルダー・保存名・操作に応じた拡張子filterを渡します。選択中は競合する編集・追加I/O・終了を防ぎ、キャンセルではproject・保存先・dirty状態を維持します。保存後のOpen／終了の継続はキャンセル・失敗で解除し、失敗を表示して再操作できます。OS固有コードはアプリ・音声コアへ持ち込みません。

Linuxのnative backendは今回未実装で`Unsupported`を返し、既存のアプリ内ブラウザーを使用します。ScarletUIのSWS backendは既存Files/sbusのsingle open/save request/replyを再利用しますが、providerに複数選択・任意extension filter・caller window・remote cancelがありません。Resonaraが要求する拡張子filterには`Unsupported`を返してbrowserへフォールバックします。Scarlet上のResonara本体の音声・クロスビルド対応を意味しません。

ScarletUIの固定Git revは`2e5e96a5c29086c3b555c85f7835e81ecf529290`です。Canvas resize 修正はこの公開済み revision に含まれています。SWS protocol は`vendor/sws-protocol`で Scarlet `0639a916dfd652e9b2c1ea740cacc1c09743d9eb` の version 13 に更新し、実行中 server に揃えています。SWS client と SGFX の runtime は同じ既存 pin を共有し、型とリンクの二重化を避けています。全 UI crate は同じ Git source です。`scripts/check-dependencies` が lockfile の protocol、UI、runtime pins を確認します。

## 検証

```sh
./scripts/dev bash scripts/verify
./scripts/dev bash scripts/headless bash scripts/gui-smoke
```

`verify` はGit依存の固定・同一性、フォーマット、workspaceテスト、ビルドを実行します。LinuxではさらにALSAファイル出力の音声スモークを実行します。スモーク用ALSA設定はそのプロセス内だけに適用し、システム設定は変更しません。実デバイスを使わず、CPALのコールバック進行と非ゼロの音声出力を検査します。macOSではALSAスモークを明示的にスキップし、通常起動時のCoreAudio再生を別に確認します。`headless`と`gui-smoke`はLinux専用で、Macではネイティブウィンドウを使います。GUIスモークは毎回新しい出力フォルダーを作り、過去の保存・書出し結果で誤って成功しないようにしています。`RESONARA_BIN`でGUI検証用の実行ファイルを変更できます。

`artifacts/` に検証ログ、保存・再読込プロジェクト、書出しWAV、再生音声、画面PNGを生成します。追加のファイルdialogテストでは4操作の結果配送、dirty sessionでのキャンセル、invalid path・失敗・retry、pending picker中の編集／I/O／終了防止、未対応時のbrowserを確認します。macOSの専用probeは共通APIのsingle/multi/save・キャンセル・owner終了を、Resonara本体は別プロセスのGUIで4導線の表示・選択・キャンセルを確認しています。SWSはstd/legacyのtarget compileまでで、Scarlet実機のサービス応答は未検証です。

テストではミックス値、Mute/Solo/Pan/Master、分割後の44.1/96kHz再生一致、保存・書出し、各PCM幅、不正データ、Undo/Redo、キャンセル、非同期I/O、テキスト入力とショートカット、フェーダー、ElementTree上のポインター配送を検証します。スレッド単位の計測allocatorでrenderを1000回実行し、確保・解放ゼロを検査します。

5分4秒の実録音を使った8トラック負荷試験のライセンス、再現方法、測定値は[実音声テスト](docs/audio-fixture.md)を参照してください。音声データと生成物はGit管理対象外です。CPUミックス性能、UIモデル・レイアウト性能、画面上の動作確認、実機オーディオ性能は区別して記録します。

2026-10-01のLinuxクラウド検証では固定flakeの評価に成功しましたが、コンテナーがprivate mount namespaceを許可しないため`nix develop`の起動は未成功です。同じ固定Scarlet Rustを直接起動した環境ではビルド・テスト・ALSA出力に成功しています。制限された環境でのGUI検証には許可された既存デスクトップを使い、セキュリティ設定を変更していません。

Mac移行では既存Nix経由のdebug/releaseビルドと125テストが通過し、Apple M3 ProのMetal GPUで起動を確認しました。ユーザーの操作でCoreAudioの出音も確認済みです。ScrollView操作の重さ、途中起動の汎用`UI: RenderError`は未解決で、実機フレーム間隔・レイテンシ・ドロップアウトは未計測です。[検証記録](docs/validation.md)と[UI計測の限界](docs/ui-performance.md)を参照してください。
