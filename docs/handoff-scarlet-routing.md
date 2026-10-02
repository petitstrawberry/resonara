# Resonara / Scarlet 引き継ぎ（2026-10-02）

## 無料 TAL-Reverb-4 の macOS 導入（同日）

- ユーザーの選択で [公式 macOS 配布](https://tal-software.com/products/tal-reverb-4) から CLAP 4.0.4 を導入。署名・notarization・bundle signature を確認し、CLAP payload のみ `~/Library/Audio/Plug-Ins/CLAP/TAL-Reverb-4.clap` に配置した。Intel / arm64 universal binary。アプリへの vendor binary 同梱や Scarlet への配置はしていない。
- catalog の名前は `TAL Reverb 4 Plugin`、ID は `ch.toguaudioline.talreverb4`。20 parameters / 641 bytes state。Cocoa embedded GUI negotiation に成功（実 window 表示は未検証）。
- Resonara Engine で3秒の wet impulse response と50 ms後の tail energy = 0.008938697256020839 を確認。state save/reopen、WAV export、正確な dry bypass、live bypass 時の同一 Engine 維持が成功。ログは `artifacts/tal-reverb4/inspect.log`、`artifacts/tal-reverb4/core-smoke.log`。検証 probe / project / WAV も同じ artifact directory にある。
- アプリでは空 Insert → `Installed CLAP effects…` → `Rescan` → `TAL Reverb 4 Plugin` から追加できる。

## CLAP bypass の GUI / DSP 保持（同日）

- CLAP bypass も built-in と同じ atomic 制御に変更。graph / PluginOwner / GUI を交換せず、音声だけを dry passthrough にする。内部 DSP history は保持して凍結する。bypass 中も GUI の pending params flush は同じ audio thread で処理する。
- 最初から bypass の insert も live graph には用意し、停止中 GUI は enabled / bypass 共に同じ paused CPAL session を使う。bypass の Undo / Redo でも editor を閉じない。plugin state の変更や削除など、実際の graph rebuild は引き続き editor を閉じる。
- missing CLAP の placeholder は保持するが、bypass 中は unavailable 警告や export の必須 effect に数えない。警告は control thread で編集直後に更新する。
- 実 Gain fixture で initially bypassed の owner 保持、繰り返す dry / wet 切り替え、callback の allocation / deallocation なしを確認。app の editor pin 回帰テストで停止中・再生中の bypass と Undo / Redo を確認。331テスト成功（長時間 stress のみ除外）、`artifacts/plugin-bypass-tests.log`。この変更後の実 GUI 操作は未再確認。
- macOS debug build、Scarlet AArch64 / RISC-V64 release build と ELF 監査に成功。ログは `artifacts/plugin-bypass-build.log`、`artifacts/plugin-bypass-native-verify.log`。既存 macOS app bundle の実行ファイルも更新済み。起動中 VM / image は変更していない。

## macOS CLAP GUI の transport 寿命修正（同日）

- インストール済み Pro-Q 4 の Cocoa GUI はユーザーが表示・操作を確認。SGFX の main window とは別の host NSWindow / NSView に埋め込む。
- GUI が閉じる原因は `play()` の inactive editor close、`finish_audio()` の全 editor close、`seek()` の Stop→Play。重なり順の修正だけでは解決しなかった。
- 停止中 GUI も paused CPAL session の同じ PluginOwner を使う。GUI が開いている間は Stop / EOF 後も session を保持し、Play はその session を再開する。停止中は無音の callback で active CLAP の pending params flush を処理する。
- desktop の seek / resume は audio block 境界の atomic request。DSP history は reset するが GUI / activation / owner を破棄しない。graph handoff でも request を共有する。Scarlet SAS は従来の drain / restart 経路を維持する。
- 別 project の Open、insert の置換・削除など graph rebuild、app close は editor の寿命を終える。Scarlet 用 CLAP GUI ABI は引き続き未実装。
- 回帰テストは stopped editor→Play、Stop→seek→Play、EOF→Play、繰り返し Stop、paused CLAP flush / reset の start/stop 回数、seek と graph handoff の競合を含む。328テスト成功（長時間 stress のみ除外）。ログは `artifacts/plugin-transport-tests.log`。この transport 修正後の実 GUI 操作はまだユーザー再確認待ち。

## 手元での再生継続修正（同日、クラウド引き継ぎ後）

- 名前変更、built-in bypass、通常の編集、Undo/Redo、Import、Save、Export は再生を継続する。
- CPAL/SAS の出力を閉じず、構造変更は control thread で準備して音声 block 境界で交換する。小数を含む再生位置を引き継ぎ、古い Engine と CLAP owner は control thread で回収する。
- built-in / CLAP bypass は atomic 制御で、effect の内部状態を保持して凍結する。汎用 popup の CLAP parameter / state 編集と構造変更は新しい DSP 状態になる。native GUI の変更は同じ instance に反映する。crossfade／click-free は未実装。
- Region の選択と drag は再生位置を維持し、drag の変更は release 時に反映する。空白クリックと scissors は明示的 seek。
- Stop、別 project の Open、自然 EOF、音声エラーでは停止する。以下のクラウド検証結果と native ELF snapshot はこの修正前の記録であり、この修正後の Scarlet ゲスト再検証は未実施。

## 手元の検証と黒いリージョンの修正（同日）

- macOS の実 CoreAudio で、同じ stream を保持した7回の live edit と CLAP 付き再生 smoke に成功。workspace の実 CLAP fixture を含む324テスト、Gain 10テストに成功（長時間 stress は除外）。これらは SWS dependency 更新前の記録。
- ユーザーの Scarlet 起動では音が出たが、リージョンの色／波形／grid が表示されなかった。黒い部分は Canvas placeholder の `(6, 8, 14)` と一致。
- 原因は旧 SWS client version 11 と実行中 server version 13 の不一致。ScarletUI の exact-match gate がアプリを CPU renderer に落とし、CPU renderer は Canvas extension を無視していた。SWS ログの GPU compositor 正常起動とは別の判定。
- アプリの UI pin を `2e5e96a5c29086c3b555c85f7835e81ecf529290` に更新。resize 修正は upstream に取り込まれたため旧 renderer vendor を削除。
- upstream はまだ SWS protocol version 11 を指定しているため、`vendor/sws-protocol` で Scarlet `0639a916dfd652e9b2c1ea740cacc1c09743d9eb` の version 13 に更新。SWS client と SGFX の runtime は既存の同一 pin を共有する。描画コードと exact-match 判定は変更していない。
- 更新後の実 CLAP fixture を含む workspace 313テスト（うち protocol 47）、upstream renderer 58テストに成功。長時間 stress のみ除外。macOS CoreAudio の7 live edits smoke、pins、format、diff check も成功。
- AArch64 / RISC-V64 の最新 release は build と ELF 監査に合格。Scarlet checkout の Rust `a5a166ab0ba10eaad36eb90d1e4af26eadfdec0c` を使用。監査 helper は macOS の `llvm-readelf` も使えるよう更新し、既存9テストも成功。
- AArch64 Gain も build と ELF 監査に合格。native app は `target/aarch64-unknown-scarlet/release/resonara`、Gain は `artifacts/sws-update-gain-aarch64/staging/system/plugins/resonara-gain.clap`。検証ログは `artifacts/sws-update-native-verify.log`、`artifacts/sws-update-final-tests.log`、`artifacts/sws-update-renderer-tests.log`、`artifacts/sws-update-coreaudio-smoke.log`。
- ゲストの既存 image と起動中 VM は変更していない。更新した binary での表示確認は別途必要。

## Scarlet オーディオ deadline の試験導入（同日）

- ユーザーが protocol 更新後の GPU 描画と負荷改善を確認。音声 producer の遅れが疑われるため scheduler API を調査した。
- Scarlet の `scarlet_os::scheduler` は current-task の Deadline 予約を提供し、SAS 自身も output period ごとに25%の runtime を予約している。
- Resonara の SAS producer のみ、256 frames / 48 kHz に合わせて period = deadline = 5,333,333 ns、runtime = 2,666,666 ns（50%）を予約。実行中 CPU に固定する。UI thread は変更しない。
- 予約失敗時は元の policy で継続しログ出力。`RESONARA_SCARLET_DEADLINE=0` で無効化して比較できる。終了時に kernel の miss / overrun を一度だけログし、元の policy を復元。自然 EOF 後の idle SAS connection は reservation を保持しない。
- ログは `[Resonara audio] deadline enabled` / `deadline unavailable` / `deadline stats`。kernel の miss / overrun は SAS underrun 回数ではない。今回のコードでのゲストの改善効果は未測定。
- Deadline 版の AArch64 / RISC-V64 release build と ELF 監査、既存 PCM adapter 7テストに成功。ログは `artifacts/deadline-native-verify.log`、`artifacts/deadline-platform-tests.log`。バイナリは各 `target/<target>/release/resonara`。

## 外部 CLAP と GUI の準備（同日）

- 空き Insert の `Installed CLAP effects…` から、インストール済み native CLAP を一覧・Rescan・追加できる。同一 library の複数 plugin ID は別項目。標準 CLAP 配置先、絶対パスの `CLAP_PATH`、executable 横の `plugins`、Scarlet の `/system/plugins` を再帰検索。
- 同梱 Gain 固定だった core の解決を拡張。プロジェクトには basename と plugin ID のみ保存し、配置先から解決する。同名の別ファイルは曖昧として拒否。symlink cycle と canonical 重複を処理し、検索量を制限。UI 描画中は inventory を再帰走査し直さない。
- macOS の `.clap` bundle は CoreFoundation で宣言された executable を解決。entry init は bundle path、library registry は canonical binary を使用。
- 汎用 parameter popup はスクロール可能。hidden を除外し、read-only は表示だけ。旧プロジェクトの flags は false を既定にする。Apply は1個の inactive instance でまとめて編集・state 保存。
- GUI も前提にし、`HostPlugin::gui_support(api)` に owner-thread の embedded/floating negotiation を追加。**独自 GUI の表示はまだ未実装**。同じ DSP instance と GUI の所有権、active parameter queue、main callback、Scarlet の SWS window ABI を [GUI contract](clap-gui.md) に整理。SWS を Cocoa/X11 の handle として偽装しない。
- 外部ファイルとして配置した実 Gain の discovery → add → parameter → bypass → undo で再生継続、state save/reopen、exact DSP、export、callback allocation audit に成功。macOS bundle の executable 名が bundle と異なるケースも成功。
- host workspace 323テスト成功（長時間 stress のみ除外）、AArch64 / RISC-V64 release build と ELF 監査成功。ログは `artifacts/clap-discovery-tests.log`、`artifacts/clap-discovery-native-verify.log`。ゲストの外部 CLAP と独自 GUI の実機動作を確認したものではない。
- Scarlet Rust shell の macOS test run は panic runtime error で abort したため、host 用 Resonara Nix shell で再ビルドして全体成功。native は Scarlet 用 shell で検証した。

## まず結論

- 作業ブランチ: `feat/scarlet-routing`（`main` へのマージ、PR 作成は行わない）
- ルーティング、コンパクトな Insert/Send UI、最初の CLAP Gain、Scarlet 用 SWS/SAS バックエンドまで実装済み
- Linux ホストの GUI・音声・実 CLAP の動作は検証済み。最新の M/S 中央配置もホストのビルドと回帰テストは通過
- **クラウドでは Scarlet ゲスト未起動。手元ではユーザーが GPU 描画と音声を確認。今回の Deadline 予約の改善効果とゲストでの CLAP・ファイル操作は未検証**
- クラウド時点の AArch64 / RISC-V64 native ELF は M/S 配置調整前の記録。今回の最新 build の記録は上の手元の検証を参照
- クラウドでの追加作業は停止。容量確保の承認は得ておらず、キャッシュは削除していない

## 実装したもの

### Routing と UI

- Track / Aux / Group bus、Main Output、Pre/Post Pan send、Insert をプロジェクトに保存。再生と WAV export は同じグラフを使う
- Built-in Insert: Gain、OnePole、Delay。分岐しても同じ Insert の DSP は一度だけ処理する
- Bus の経路と Aux の受け口を分けた操作。Output または空 Send 行の `New Bus → Aux` は作成・接続を一度に行う。手動の `+Aux` も用意
- Insert は連続したスロット。名前でエディター、電源で bypass、右クリック／矢印から並べ替え・削除。Send は連続した小さい行とノブ、空行から行き先を選ぶ
- Inspector は左全高。リージョン情報をチャンネルの上に置き、上側だけスクロール、下の共通フェーダーは固定。Mixer と同じ channel strip を使い、M/S を中央配置
- 音量・Pan・Send level と built-in / CLAP bypass は atomic な live control。構造変更は出力を維持してグラフ交換する（手元での修正を参照）
- Undo/Redo、旧 version-1 JSON の読込、Bus 削除時の参照修復、循環・資源上限の検証を実装

### 最初の CLAP

- 同梱 `org.resonara.gain` と検索で見つかる native stereo CLAP effect に対応。パラメーターと opaque state を保存・復元し、汎用エディターで編集する
- stereo float32、Gain 0〜2。custom GUI、MIDI、sidechain、PDC、automation、可変ポート構成は未対応
- **OS/CPU が一致する native C ABI が対象。Linux `.so` を Scarlet や macOS でそのまま動かす機能ではない**
- 不明／不足プラグインは state を残して再生時に警告つき passthrough。必要な effect が使えない export は WAV 作成前にエラーにする
- JSON 内の library 値は識別子であり、その文字列を `dlopen` のパスにはしない。明示した `RESONARA_CLAP_LIBRARY` は既存の絶対パスが必要
- 所有権・activate/deactivate/destroy は作成元スレッド、音声処理は realtime proxy。停止時は CPAL stream の終了／SAS worker の join 後に owner を解放する

### Scarlet port

- Host: ScarletUI `platform-winit` + CPAL。Scarlet: `platform-sws` + `sas-client`
- SAS は 48 kHz / stereo S16LE、256-frame producer、1,024-frame ring。partial write、backpressure、cancel、timeout を扱う
- 自然終了では最終 PCM を渡したあと接続を保持し、次の明示的な transport/edit 操作で閉じる。SAS の client ring が空でもハードウェア出力完了とは限らないため
- file I/O、JSON、デコード、export は共通実装。Scarlet の file provider が拡張子 filter を満たせない場合はアプリ内 browser を使う

## 手元で Host を起動する

以下は **新しい作業ディレクトリ**で行う例。既存の変更がある checkout を上書きしないこと。Nix が利用できる前提。

```sh
git clone --branch feat/scarlet-routing https://github.com/petitstrawberry/resonara.git
cd resonara
./scripts/dev cargo run --locked --release -p resonara
```

`scripts/dev` はリポジトリの pinned Nix shell を使う。既存の適合する Rust / システム依存が揃っていれば、中の Cargo コマンドを直接実行してもよい。

### Host 用 Gain を読み込む

```sh
PLUGIN_TARGET="$PWD/plugins/resonara-gain/target"
CARGO_TARGET_DIR="$PLUGIN_TARGET" ./scripts/dev cargo build --release --locked \
  --manifest-path plugins/resonara-gain/Cargo.toml

# macOS
export RESONARA_CLAP_LIBRARY="$PLUGIN_TARGET/release/libresonara_gain.dylib"
# Linux では、上の export の代わりにこちら
# export RESONARA_CLAP_LIBRARY="$PLUGIN_TARGET/release/libresonara_gain.so"

./scripts/dev cargo run --locked --release -p resonara
```

空の Insert slot から `Resonara Gain` を選ぶ。macOS の上記 `.dylib` 手順はこのクラウドでは未実行。Linux の実 library ロード・DSP・state 復元は検証済み。

環境変数を使わない場合は、アプリ実行ファイルの隣の `plugins/resonara-gain.clap`、次に `/system/plugins/resonara-gain.clap` を探す。一般的なプラグインフォルダーの自動スキャンは実装していない。

## 手元で Scarlet を続ける

### 再現に使った revision

- Resonara の開始点: `7564ddfbeed6f3700579398883bd5dad9d3ce4bf`
- Scarlet `dev`: `0639a916dfd652e9b2c1ea740cacc1c09743d9eb`
- Scarlet Rust: `a5a166ab0ba10eaad36eb90d1e4af26eadfdec0c`、`rustc 1.94.0-nightly`、LLVM / LLD 21.1.8
- ScarletUI: `cdd852eb22678b7b0d1c7e035c1f2b2ba4d057a7`
- SAS/client 側 Scarlet dependency: `b3d2a55740a3d2ca49daad0ec7baba233f706f7a`
- QEMU fork: `d94a1407ab9ccd60559bfd80182a81bb4261fb84`、version 11.1.0

`Cargo.lock` と Git pins を維持。現在は upstream resize 修正を含む UI pin と、`vendor/sws-protocol` の protocol 更新 patch を使う。Gain 側の `vendor/clap-sys` は MIT ライセンスの no_std ABI subset。マシン固有の Cargo `[patch]` やクラウドの絶対パスを入れる必要はない。

### Native build → image → 起動

Resonara の親ディレクトリに、再現用の新しい Scarlet checkout を作る例:

```sh
git clone --branch dev https://github.com/petitstrawberry/Scarlet.git ../Scarlet
git -C ../Scarlet checkout 0639a916dfd652e9b2c1ea740cacc1c09743d9eb
SCARLET="$(cd ../Scarlet && pwd)"
RESONARA="$PWD"

# 任意: 先に native 両 target の build / ELF audit を実行
(cd "$SCARLET" &&
  nix --extra-experimental-features 'nix-command flakes' develop \
    --accept-flake-config --no-write-lock-file \
    -c bash -c 'cd "$1"; bash scripts/verify-scarlet' _ "$RESONARA")

# この profile を明示。省略時は従来の full Debian/Wine image
bash scripts/scarlet-image "$SCARLET" --profile native-desktop
bash scripts/scarlet-run "$SCARLET" --profile native-desktop --no-build
```

`SCARLET` と `RESONARA` は手元の checkout の絶対パス。別の配置でも、この2つを合わせればクラウド固有のパスは不要。

- `native-desktop` は Scarlet の exact desktop bundle を使う別プロジェクト `projects/aarch64-limine-resonara-native` を生成する。元の `aarch64-limine-full` は変更しない
- SWS/SAS、native apps、fonts、services、BSP は維持。Debian/Wine、experimental、ゲスト内 Rust toolchain は含めない。Mozc の Linux server は無いため変換不可。native SKK は残る
- rootfs は最小 2 GiB、SDK が必要に応じて拡大。GPT は約 2.06 GiB 以上。**そのほか staging、ext2、複数 Cargo cache が必要なので、2 GiB の空きで作れるわけではない**
- build helper は現在のアプリと Gain をビルド・監査してから `/bin/resonara` と `/system/plugins/resonara-gain.clap` に配置する
- Native の `dl*` は resident `/bin/scarlet-ld` が供給する。別の `scarlet-dl` を静的に埋め込まない。PIE/PIC と限定した link seed を使用し、**監査なしの ELF を配置しない**
- 通常の run は既存 disk を再利用。再生成は `--replace-image` が必要。ゲストに保存したものを先に取り出すこと。独立して起動した別 VM は自動停止しない
- 既定は TCG、4 GiB、4 CPU、GL GPU、network off、WAV audio capture。Mac は公式 runner の Cocoa GL を選択する。通常の VNC / non-GL GPU は使わない。HVF は今回未検証
- 音声 capture は `artifacts/scarlet-native/vm/audio.wav`、serial は同じ場所の `serial.log`。ホストのスピーカーで聞こえることは別確認

詳しくは [Scarlet port](scarlet-port.md)、[native profile](../platforms/scarlet/native-desktop/README.md)、[CLAP contract](clap-host.md)。

## 検証結果と残り

### 通ったもの

- **最新 M/S ソースで最終 aggregate: workspace 301 passed / 10 default-ignored**。そのうち fixture が必要な9ケースも明示して成功。既存の長時間 stress workload だけ未実行
- 実 library の追加検証は host 5 / core 9 / app 1 成功（通常テストとの重複を含む）。Gain 10 ケース、ELF auditor 9 ケースも成功
- app 単体は **153 passed**。pins、format、doc tests、host build、diff check も成功
- Linux CPAL/ALSA: flat / routed / routed+CLAP の save-load-export-playback smoke に成功。最終 capture は flat 529,200 bytes、routed / routed+CLAP 各458,640 bytes の nonzero PCM（長さは callback のタイミングで変わる）
- AArch64 / RISC-V64 native ELF: `dlopen/dlsym/dlclose/dlerror` の4 imports、`DT_NEEDED` / TLS なし、対応する `JUMP_SLOT` / `RELATIVE` のみで監査成功
- クラウドの実 desktop で Nix 2.24.12 rootless store と pinned QEMU 11.1.0 をビルド・実行。40-library のローカル Mesa closure により **実際の GTK GL window** を表示

Native ELF の M/S 調整前 snapshot:

- AArch64: `dec6dd1c80631d8a6081c2a51958003a5a6ecde393b96b10a73489922717bb02`
- RISC-V64: `92b05746599b6f2ae8350900329e274e12497b42cc394535296e0f25ec0270b0`

最終ホスト検証ログは `artifacts/routing/final-handoff-host-verify.log`、終了コード0。これらのクラウド実行ログは Git 対象外。

QEMU の画面確認は **kernel を載せていない、停止中の 128 MiB guest**。Scarlet の起動確認ではない。

### 手元で次に確認する順序

1. 現在のソースを native build / ELF audit し、image を作る
2. QEMU で Scarlet を boot。SWS 上に Resonara を開き、resize、Import、Inspector/Mixer を確認
3. Gain を load し、0.5 の DSP、parameter/state の save/reopen、bypass を確認
4. SAS 再生と停止・seek・自然終了を確認し、captured WAV が nonzero か調べる
5. ゲスト内で project を Save → Reopen → Export し、再起動後も保持されるか確認

### クラウド側で止めた地点（手元の必須設定ではない）

8 GiB memory / 32 GiB disk の環境。full devShell 評価は OOM で止まり、個別の pinned derivation を1 jobで実現した。QEMU と GL window は成功したが、その後の SDK/firmware 準備で空きが約 2 GiB まで減少したため停止した。現在の残りは SDK/firmware の6 derivations。`cargo-scarlet` の失敗時 scratch は保持した。

クラウド固有の bootstrap、Mesa copy、絶対パス、未完了 Nix store は、この branch の実行条件にしていない。build logs、cross binaries、images、screenshots、toolchains は Git に含めない。小さい codec fixture は自作の合成音で、テストのため source に含める。
