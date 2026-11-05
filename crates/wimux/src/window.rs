//! メインウィンドウ: サイドバー（ワークスペース一覧）・タブバー・分割ペイン・ポップアップ。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use std::rc::Rc;

use crate::browser::{self, Browser, BrowserMsg, BrowserSender};
use crate::dirs::DirHistory;
use crate::notify;
use crate::picker::{Picker, PickerEvent, PickerKind};
use crate::session::{self, Axis, NodeState, PaneState, SessionFile, WorkspaceState};
use crate::terminal::osc::ShellEvent;
use crate::terminal::view::{TerminalEvent, TerminalView};
use crate::terminal::{SpawnOptions, Terminal};
use crate::text_field::{TextField, TextFieldEvent};
use crate::workspace::{Direction, Group, GroupId, Node, PaneId, Workspace};

gpui_kit::actions!(
    wimux,
    [
        NewWorkspace,
        NextWorkspace,
        PrevWorkspace,
        RenameWorkspace,
        NewTab,
        NextTab,
        PrevTab,
        ClosePane,
        SplitRight,
        SplitDown,
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        ResizeLeft,
        ResizeRight,
        ResizeUp,
        ResizeDown,
        SelectWorkspace1,
        SelectWorkspace2,
        SelectWorkspace3,
        SelectWorkspace4,
        SelectWorkspace5,
        SelectWorkspace6,
        SelectWorkspace7,
        SelectWorkspace8,
        SelectWorkspace9,
        OpenDirPicker,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ToggleSidebar,
        OpenSettings,
        OpenBrowser,
        JumpToAttention,
        SplitBrowser,
        CommandPalette,
    ]
);

use crate::keymap::CONTEXT;

use crate::settings::{self, Backdrop, CursorKind, Settings};
use crate::terminal::view::installed_fonts;
use crate::theme::{self, *};

/// AI エージェント（Claude Code・Codex など）の状態。
/// Claude Code はフック（シェル連携の `claude` 関数から起動したとき）かタイトル、
/// ほかのエージェントは出力の動き（＋ベル・通知、Codex は notify）から推定する
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentState {
    None,
    /// 応答が終わって待機中
    Idle,
    /// 応答を生成中
    Working,
    /// 許可・質問への回答を待って止まっている（フック・ベル・通知で検知）
    Asking,
}

const CLAUDE: &str = "Claude";

/// シェル連携のラッパー・フックが送ってくるコマンド名 → 表示名
fn agent_label(name: &str) -> Option<&'static str> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "claude" => CLAUDE,
        "codex" => "Codex",
        "gemini" => "Gemini",
        "aider" => "Aider",
        "opencode" => "OpenCode",
        "copilot" => "Copilot",
        "cursor-agent" => "Cursor",
        "qwen" => "Qwen",
        "amp" => "Amp",
        "crush" => "Crush",
        "goose" => "Goose",
        "kiro-cli" | "kiro" => "Kiro",
        _ => return None,
    })
}

/// 表示名 → シェルのコマンド名（session.json に保存する名前）
fn agent_command(label: &str) -> Option<&'static str> {
    Some(match label {
        "Codex" => "codex",
        "Gemini" => "gemini",
        "Aider" => "aider",
        "OpenCode" => "opencode",
        "Copilot" => "copilot",
        "Cursor" => "cursor-agent",
        "Qwen" => "qwen",
        "Amp" => "amp",
        "Crush" => "crush",
        "Goose" => "goose",
        "Kiro" => "kiro-cli",
        _ => return None,
    })
}

/// Claude 以外のエージェントの、前回の会話を再開するコマンド（再開の方法が分かるものだけ）
fn agent_resume_command(command: &str, session: Option<&str>) -> Option<String> {
    Some(match command {
        "codex" => match session {
            Some(id) => format!("codex resume {id}"),
            // そのフォルダでいちばん新しい会話
            None => "codex resume --last".into(),
        },
        "opencode" => "opencode --continue".into(),
        "copilot" => "copilot --continue".into(),
        "gemini" => "gemini --resume latest".into(),
        _ => return None,
    })
}

/// 通知・サイドバーに出すエージェント名
fn agent_title(agent: Option<&'static str>) -> &'static str {
    match agent {
        Some(CLAUDE) | None => "Claude Code",
        Some(a) => a,
    }
}

/// 出力が止まってからこれだけ経ったら「待機中」とみなす（出力の動きで推定するエージェント）
const AGENT_QUIET: Duration = Duration::from_millis(2000);
/// これより長く動いていたら、止まったときに「完了」を通知する
const AGENT_MIN_WORK: Duration = Duration::from_secs(4);

/// ペインの状態の色。
/// 実行中=黄、入力待ち（許可・質問）=赤、完了して未確認=緑、確認済みの待機=灰、その他の通知=青
fn pane_dot(claude: AgentState, attention: bool) -> Option<u32> {
    let c = th();
    match (claude, attention) {
        (AgentState::Asking, _) => Some(c.red),
        (AgentState::Working, _) => Some(c.amber),
        (AgentState::Idle, true) => Some(c.green),
        (AgentState::Idle, false) => Some(c.text_faint),
        (AgentState::None, true) => Some(c.accent),
        (AgentState::None, false) => None,
    }
}

/// 複数ペインのうち、いちばん目を引くべき状態（ワークスペースの丸に使う）
fn dot_priority(claude: AgentState, attention: bool) -> u8 {
    match (claude, attention) {
        (AgentState::Asking, _) => 5,
        (AgentState::Idle, true) => 4,
        (AgentState::None, true) => 3,
        (AgentState::Working, _) => 2,
        (AgentState::Idle, false) => 1,
        (AgentState::None, false) => 0,
    }
}

/// Claude Code はタイトルを「✳ Claude Code」（待機中）や、先頭がスピナー文字（作業中）にする
fn claude_state_from_title(title: &str) -> Option<AgentState> {
    let first = title.chars().next()?;
    let is_spinner = ('\u{2800}'..='\u{28FF}').contains(&first) || "·✢✻✽✶*".contains(first);
    if title.contains("Claude") || first == '✳' {
        Some(if is_spinner {
            AgentState::Working
        } else {
            AgentState::Idle
        })
    } else {
        None
    }
}

/// ドラッグ中のタブ
#[derive(Clone)]
/// ドラッグ中の分割の境界（GPUI のドラッグの仕組みを使う）。根からの道で分割を特定する
struct DraggedDivider {
    path: Vec<bool>,
}

/// 境界のドラッグ中にマウスについてくる見た目（何も出さない）
struct DividerPreview;

impl Render for DividerPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

struct DraggedTab {
    pane: PaneId,
    label: String,
}

/// ドラッグ中のワークスペース（サイドバーで並べ替え）
#[derive(Clone)]
struct DraggedWorkspace {
    index: usize,
    label: String,
}

/// ドラッグ中にマウスについてくる表示
struct TabDragPreview {
    label: String,
}

impl Render for TabDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let c = th();
        div()
            .px(px(12.))
            .py(px(5.))
            .rounded(px(7.))
            .bg(rgb(c.hover_bg_strong))
            .border_1()
            .border_color(rgb(c.accent))
            .text_size(px(UI_TAB))
            .text_color(rgb(c.text))
            .child(self.label.clone())
    }
}

/// 区画のどこに落とすか（中央 = その区画のタブに加える、端 = 分割して置く）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropZone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

impl DropZone {
    /// 区画内の位置（0〜1）から。端から 25% 以内なら、いちばん近い辺
    fn at(x: f32, y: f32) -> Self {
        let edges = [
            (x, DropZone::Left),
            (1. - x, DropZone::Right),
            (y, DropZone::Top),
            (1. - y, DropZone::Bottom),
        ];
        let (d, zone) = edges
            .into_iter()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap_or((1., DropZone::Center));
        if d < 0.25 { zone } else { DropZone::Center }
    }
}

/// タブの移動先
#[derive(Debug, Clone, Copy)]
enum TabDest {
    /// 区画のタブの並びの index 番目（usize::MAX なら最後）
    Tab { gid: GroupId, index: usize },
    /// 区画の中央（タブとして加える）か端（分割して置く）
    Zone { gid: GroupId, zone: DropZone },
    /// サイドバーのワークスペース（フォーカスしている区画のタブとして加える）
    Workspace(usize),
    /// サイドバーの空いているところ（新しいワークスペースにする）
    NewWorkspace,
}

struct Pane {
    view: Option<Entity<TerminalView>>,
    /// 起動時の作業フォルダ（遅延起動のため保持）
    spawn_cwd: Option<String>,
    /// 起動後、プロンプトが出たら順に流すコマンド（復元用）
    restore_cmds: Vec<String>,
    cwd: Option<String>,
    conda: Option<String>,
    title: Option<String>,
    state: AgentState,
    /// 動いているエージェントの表示名（"Claude" / "Codex" など）
    agent: Option<&'static str>,
    /// 状態をフックから受け取っている（Claude Code）。タイトル・ベルからの推定は控えめにする
    hooked: bool,
    /// 状態を出力の動きから推定する（フックのないエージェント）
    activity_driven: bool,
    working_since: Option<Instant>,
    asking_since: Option<Instant>,
    /// エージェントのセッション ID。Claude Code は前回から引き継いだもの（このペインでまだ再開していない
    /// 場合に使う）、Codex は notify から届いた thread-id
    agent_session: Option<String>,
    /// git のブランチ名（cwd から .git/HEAD を読む）
    branch: Option<String>,
    /// 見ていない間に通知があった（ペインに青い枠を付ける）
    attention: bool,
    /// 起動後に一度でもプロンプトが出たか（復元直後のフォルダも履歴に記録するため）
    prompted: bool,
    last_notified: Option<Instant>,
    _sub: Option<Subscription>,
    /// ブラウザのタブなら開いている URL（None ならターミナル）
    url: Option<String>,
    /// ブラウザ本体（表示されたときに作る）
    browser: Option<Rc<Browser>>,
    /// ブラウザを作れなかったときの理由（何度も作り直さないため）
    browser_error: Option<String>,
    /// ブラウザを作成中
    browser_pending: bool,
    /// URL 欄
    url_field: Option<Entity<TextField>>,
    _field_sub: Option<Subscription>,
    loading: bool,
}

impl Pane {
    fn new_browser(url: String) -> Self {
        let mut p = Self::new(None);
        p.url = Some(url);
        p
    }

    fn is_browser(&self) -> bool {
        self.url.is_some()
    }

    fn new(cwd: Option<String>) -> Self {
        Self {
            view: None,
            spawn_cwd: cwd.clone(),
            restore_cmds: Vec::new(),
            cwd,
            conda: None,
            title: None,
            state: AgentState::None,
            agent: None,
            hooked: false,
            activity_driven: false,
            working_since: None,
            asking_since: None,
            agent_session: None,
            branch: None,
            attention: false,
            prompted: false,
            last_notified: None,
            _sub: None,
            url: None,
            browser: None,
            browser_error: None,
            browser_pending: false,
            url_field: None,
            _field_sub: None,
            loading: false,
        }
    }
}

enum PickerTarget {
    Rename(usize),
    /// コマンドパレット
    Command,
}

pub struct WimuxWindow {
    workspaces: Vec<Workspace>,
    active: usize,
    panes: HashMap<PaneId, Pane>,
    groups: HashMap<GroupId, Group>,
    /// ペイン・区画の ID の払い出し
    next_id: usize,
    history: DirHistory,
    picker: Option<(Entity<Picker>, PickerTarget, Subscription)>,
    save_task: Option<Task<()>>,
    closing: bool,
    /// タブをドラッグ中、どの区画のどこに落とすか（区画の上に予告を出す）
    drop_hint: Option<(GroupId, DropZone)>,
    focus_handle: FocusHandle,
    /// ターミナルの文字サイズ（Ctrl+= / Ctrl+- / Ctrl+0）
    font_size: f32,
    settings: Settings,
    /// 設定画面を開いているか
    settings_open: bool,
    settings_focus: FocusHandle,
    /// ブラウザ用の共有設定（データ保存先）。最初のブラウザを開くときに作る
    web_context: Option<Rc<std::cell::RefCell<wry::WebContext>>>,
    browser_tx: BrowserSender,
    sidebar_visible: bool,
    /// 右クリックメニュー（対象のワークスペース番号, 表示位置）
    ws_menu: Option<(usize, Point<Pixels>)>,
    /// 分割の境界をマウスでドラッグ中（タブのドラッグと区別するため）
    resizing: bool,
    /// アップデートの状態（サイドバーの下にお知らせを出す）
    update: UpdateState,
    /// 利用上限（5 時間枠・週）。Claude のステータスライン・Codex の notify から届く（届いた順）
    usage: Vec<(&'static str, AgentUsage)>,
    /// 利用上限の「あと何分」を描き直した分（1 分ごとに描き直す）
    usage_minute: i64,
    _subs: Vec<Subscription>,
}

/// エージェントの利用上限（届いた最新の値）
#[derive(Debug, Clone, Copy, Default)]
struct AgentUsage {
    five_hour: Option<crate::ipc::LimitWindow>,
    seven_day: Option<crate::ipc::LimitWindow>,
}

impl AgentUsage {
    fn any(&self) -> bool {
        self.five_hour.is_some() || self.seven_day.is_some()
    }
}

/// アップデートのお知らせ
#[derive(Debug, Clone, PartialEq)]
enum UpdateState {
    None,
    /// 新しい版がある
    Available(crate::update::Release),
    /// ダウンロード・インストールを始めている
    Installing(crate::update::Release),
    /// 更新できなかった（理由）
    Failed(crate::update::Release, String),
    /// お知らせを閉じた（この起動の間は出さない）
    Dismissed,
}

impl WimuxWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = settings::load();
        theme::apply(&settings.theme);
        theme::set_opacity(settings.opacity);
        theme::set_line_height(settings.line_height);
        crate::terminal::set_default_cursor(settings.cursor);
        apply_backdrop(&settings, window);
        let (browser_tx, browser_rx) = flume::unbounded::<(PaneId, BrowserMsg)>();
        let mut this = Self {
            workspaces: Vec::new(),
            active: 0,
            panes: HashMap::new(),
            groups: HashMap::new(),
            next_id: 1,
            history: DirHistory::load(&session::dirs_path()),
            settings: settings.clone(),
            settings_open: false,
            settings_focus: cx.focus_handle(),
            web_context: None,
            browser_tx: browser_tx.clone(),
            picker: None,
            save_task: None,
            closing: false,
            drop_hint: None,
            font_size: settings.font_size,
            sidebar_visible: true,
            ws_menu: None,
            resizing: false,
            focus_handle: cx.focus_handle(),
            update: UpdateState::None,
            usage: Vec::new(),
            usage_minute: 0,
            _subs: Vec::new(),
        };

        match session::load_session(&session::session_path()) {
            Some(s) => this.restore(s),
            None => {
                let gid = this.alloc_group(None);
                this.workspaces.push(Workspace::new(gid));
            }
        }

        // ウィンドウを閉じる前に保存
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            // 位置・大きさ・最大化を覚えておく（次の起動で同じように開く）
            crate::window_state::save(&crate::window_state::SavedWindow::from_bounds(
                window.window_bounds(),
            ));
            let _ = weak.update(cx, |this, _| {
                this.closing = true;
                this.save_now();
            });
            true
        });
        // ウィンドウがアクティブになったら、表示中ワークスペースの未読を消す
        let sub = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active()
                && let Some(ws) = this.workspaces.get_mut(this.active)
            {
                ws.unread = 0;
                cx.notify();
            }
        });
        this._subs.push(sub);

        // ブラウザからのお知らせ（タイトル・URL の変化）
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(first) = browser_rx.recv_async().await {
                let mut msgs = vec![first];
                msgs.extend(browser_rx.try_iter());
                let alive = this
                    .update_in(cx, |this, window, cx| {
                        for (id, msg) in msgs {
                            match msg {
                                BrowserMsg::NewWindow(url) => {
                                    this.open_new_window_from(id, url, window, cx)
                                }
                                BrowserMsg::Key(key) => this.on_browser_key(id, &key, window, cx),
                                msg => this.on_browser_msg(id, msg, cx),
                            }
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
        // ブラウザはメインウィンドウとは別のウィンドウなので、メインウィンドウを動かしたら追従させる
        if browser::SUPPORTED {
            cx.spawn(async move |this, cx| {
                let mut tick: u32 = 0;
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(30))
                        .await;
                    tick = tick.wrapping_add(1);
                    let alive = this
                        .update(cx, |this, cx| {
                            let mut changed = Vec::new();
                            for (id, pane) in &this.panes {
                                if let Some(b) = &pane.browser {
                                    b.reposition();
                                    // ページ内の移動（SPA の pushState・#）はページ読み込みの
                                    // 通知が来ないので、今の URL をときどき確かめる（約 0.3 秒ごと）
                                    if tick % 10 == 0
                                        && let Some(u) = b.current_url()
                                        && pane.url.as_deref() != Some(u.as_str())
                                    {
                                        changed.push((*id, u));
                                    }
                                }
                            }
                            if !changed.is_empty() {
                                for (id, u) in changed {
                                    this.on_browser_msg(id, BrowserMsg::Url(u), cx);
                                }
                                cx.notify();
                            }
                        })
                        .is_ok();
                    if !alive {
                        break;
                    }
                }
            })
            .detach();
        }

        // フック・`sondev notify` からの知らせ
        if let Some(rx) = crate::ipc::receiver() {
            cx.spawn_in(window, async move |this, cx| {
                while let Ok(first) = rx.recv_async().await {
                    let mut msgs = vec![first];
                    msgs.extend(rx.try_iter());
                    let alive = this
                        .update_in(cx, |this, window, cx| {
                            for msg in msgs {
                                this.on_ipc(msg, window, cx);
                            }
                            cx.notify();
                        })
                        .is_ok();
                    if !alive {
                        break;
                    }
                }
            })
            .detach();
        }
        // 新しいバージョンの確認（起動の 5 秒後、その後 12 時間ごと）
        if crate::update::auto_check_enabled() {
            cx.spawn(async move |this, cx| {
                let mut wait = Duration::from_secs(5);
                loop {
                    cx.background_executor().timer(wait).await;
                    wait = Duration::from_secs(12 * 3600);
                    let enabled = this.update(cx, |this, _| this.settings.check_updates);
                    match enabled {
                        Err(_) => break,
                        Ok(false) => continue,
                        Ok(true) => {}
                    }
                    let found = cx
                        .background_executor()
                        .spawn(async { crate::update::check() })
                        .await;
                    let alive = this.update(cx, |this, cx| {
                        if let Some(r) = found
                            && matches!(this.update, UpdateState::None | UpdateState::Available(_))
                        {
                            this.update = UpdateState::Available(r);
                            cx.notify();
                        }
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        // フックのないエージェントの状態を、出力の動きから推定する
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.tick_agents(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        this.focus_active(window, cx);
        this
    }

    // -----------------------------------------------------------------------
    // ペインの生成・起動
    // -----------------------------------------------------------------------

    fn alloc_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn alloc_pane(&mut self, cwd: Option<String>) -> PaneId {
        let id = self.alloc_id();
        self.panes.insert(id, Pane::new(cwd));
        id
    }

    /// タブ 1 つを持つ区画を作る
    fn alloc_group(&mut self, cwd: Option<String>) -> GroupId {
        let pane = self.alloc_pane(cwd);
        let id = self.alloc_id();
        self.groups.insert(id, Group::new(pane));
        id
    }

    /// ペインのシェルを（まだなら）起動する。表示されるまで起動しない＝起動が速い
    fn ensure_view(
        &mut self,
        id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<TerminalView>> {
        let font_size = self.font_size;
        let font_family = self.settings.font_family.clone();
        let pane = self.panes.get_mut(&id)?;
        if let Some(view) = &pane.view {
            return Some(view.clone());
        }
        if pane.is_browser() {
            return None;
        }
        let cwd = pane
            .spawn_cwd
            .as_ref()
            .map(PathBuf::from)
            .filter(|p| p.is_dir());
        let terminal = match Terminal::spawn(SpawnOptions {
            cwd,
            pane_id: crate::claude_session::pane_key(id),
        }) {
            Ok(t) => t,
            Err(e) => {
                log::error!("ターミナルを起動できませんでした: {e:#}");
                return None;
            }
        };
        let cmds = std::mem::take(&mut pane.restore_cmds);
        let view = cx.new(|cx| {
            let mut v = TerminalView::new(terminal, window, cx);
            v.set_font_size(font_size);
            v.set_font_family(font_family.as_deref(), cx);
            v.queue_commands(cmds);
            v
        });
        let sub = cx.subscribe_in(
            &view,
            window,
            move |this, _, ev: &TerminalEvent, window, cx| {
                this.on_terminal_event(id, ev, window, cx);
            },
        );
        let pane = self.panes.get_mut(&id)?;
        pane.view = Some(view.clone());
        pane._sub = Some(sub);
        Some(view)
    }

    fn focus_pane(&mut self, id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane) = self.panes.get_mut(&id) {
            pane.attention = false;
            if pane.is_browser() {
                // ブラウザはまだ作られていなければ次の描画で作られる。キー入力は wimux 側に残す
                self.focus_handle.focus(window, cx);
                return;
            }
        }
        if let Some(view) = self.ensure_view(id, window, cx) {
            let handle = view.read(cx).focus_handle(cx);
            handle.focus(window, cx);
            // 分割・新規タブで作ったばかりのペインは、まだ一度も描画されていないため
            // フォーカスが定着しないことがある。描画が終わった後にもう一度フォーカスする
            cx.on_next_frame(window, move |this, window, cx| {
                if this.focused_pane() == Some(id) && !handle.is_focused(window) {
                    handle.focus(window, cx);
                }
            });
        }
    }

    fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            return;
        }
        if let Some(id) = self.focused_pane() {
            self.focus_pane(id, window, cx);
        }
        cx.notify();
    }

    fn focused_group(&self) -> Option<GroupId> {
        Some(self.workspaces.get(self.active)?.focused)
    }

    fn focused_pane(&self) -> Option<PaneId> {
        self.groups.get(&self.focused_group()?)?.active_pane()
    }

    fn focused_cwd(&self) -> Option<String> {
        let id = self.focused_pane()?;
        self.panes.get(&id)?.cwd.clone()
    }

    /// ワークスペースの全ペイン
    fn ws_panes(&self, ws: &Workspace) -> Vec<PaneId> {
        ws.groups()
            .iter()
            .filter_map(|g| self.groups.get(g))
            .flat_map(|g| g.tabs.iter().copied())
            .collect()
    }

    /// ワークスペースでフォーカスしているペイン
    fn ws_focused_pane(&self, ws: &Workspace) -> Option<PaneId> {
        self.groups.get(&ws.focused)?.active_pane()
    }

    /// ペインがどのワークスペース・区画・タブ番号にあるか
    fn locate(&self, id: PaneId) -> Option<(usize, GroupId, usize)> {
        self.workspaces.iter().enumerate().find_map(|(wi, ws)| {
            ws.groups().into_iter().find_map(|gid| {
                let ti = self.groups.get(&gid)?.tabs.iter().position(|p| *p == id)?;
                Some((wi, gid, ti))
            })
        })
    }

    // -----------------------------------------------------------------------
    // 操作
    // -----------------------------------------------------------------------

    fn select_workspace(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.workspaces.len() {
            return;
        }
        self.active = index;
        self.workspaces[index].unread = 0;
        self.focus_active(window, cx);
        self.schedule_save(cx);
    }

    fn new_workspace(&mut self, _: &NewWorkspace, window: &mut Window, cx: &mut Context<Self>) {
        let gid = self.alloc_group(None);
        self.workspaces.push(Workspace::new(gid));
        self.select_workspace(self.workspaces.len() - 1, window, cx);
    }

    /// ワークスペースの並べ替え: `from` 番目を `to` 番目へ（表示中のワークスペースは変えない）
    fn move_workspace(
        &mut self,
        from: usize,
        to: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = self.workspaces.len();
        if from >= n || to >= n || from == to {
            return;
        }
        let ws = self.workspaces.remove(from);
        self.workspaces.insert(to, ws);
        self.active = moved_index(self.active, from, to);
        self.ws_menu = None;
        self.focus_active(window, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    fn next_workspace(&mut self, _: &NextWorkspace, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.workspaces.len();
        if n > 0 {
            self.select_workspace((self.active + 1) % n, window, cx);
        }
    }

    fn prev_workspace(&mut self, _: &PrevWorkspace, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.workspaces.len();
        if n > 0 {
            self.select_workspace((self.active + n - 1) % n, window, cx);
        }
    }

    fn select_ws_1(&mut self, _: &SelectWorkspace1, w: &mut Window, cx: &mut Context<Self>) {
        self.select_workspace(0, w, cx);
    }
    fn select_ws_2(&mut self, _: &SelectWorkspace2, w: &mut Window, cx: &mut Context<Self>) {
        self.select_workspace(1, w, cx);
    }
    fn select_ws_3(&mut self, _: &SelectWorkspace3, w: &mut Window, cx: &mut Context<Self>) {
        self.select_workspace(2, w, cx);
    }
    fn select_ws_4(&mut self, _: &SelectWorkspace4, w: &mut Window, cx: &mut Context<Self>) {
        self.select_workspace(3, w, cx);
    }
    fn select_ws_5(&mut self, _: &SelectWorkspace5, w: &mut Window, cx: &mut Context<Self>) {
        self.select_workspace(4, w, cx);
    }
    fn select_ws_6(&mut self, _: &SelectWorkspace6, w: &mut Window, cx: &mut Context<Self>) {
        self.select_workspace(5, w, cx);
    }
    fn select_ws_7(&mut self, _: &SelectWorkspace7, w: &mut Window, cx: &mut Context<Self>) {
        self.select_workspace(6, w, cx);
    }
    fn select_ws_8(&mut self, _: &SelectWorkspace8, w: &mut Window, cx: &mut Context<Self>) {
        self.select_workspace(7, w, cx);
    }
    /// Ctrl+9 は最後のワークスペース（ブラウザのタブと同じ）
    fn select_ws_9(&mut self, _: &SelectWorkspace9, w: &mut Window, cx: &mut Context<Self>) {
        let last = self.workspaces.len().saturating_sub(1);
        self.select_workspace(last, w, cx);
    }

    fn rename_workspace(
        &mut self,
        _: &RenameWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let index = self.active;
        self.open_rename(index, window, cx);
    }

    fn open_rename(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.workspace_name(index);
        self.open_picker(
            PickerKind::Text {
                title: "ワークスペース名（空にするとフォルダ名を自動表示）".into(),
            },
            current,
            PickerTarget::Rename(index),
            window,
            cx,
        );
    }

    fn close_workspace(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.workspaces.len() {
            return;
        }
        let ws = self.workspaces.remove(index);
        for gid in ws.groups() {
            if let Some(group) = self.groups.remove(&gid) {
                for id in group.tabs {
                    self.panes.remove(&id);
                }
            }
        }
        if self.workspaces.is_empty() {
            self.closing = true;
            self.save_now();
            crate::window_state::save(&crate::window_state::SavedWindow::from_bounds(
                window.window_bounds(),
            ));
            cx.quit();
            return;
        }
        if self.active > index || self.active >= self.workspaces.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.focus_active(window, cx);
        self.schedule_save(cx);
    }

    /// フォーカスしている区画に新しいタブを開く
    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(gid) = self.focused_group() {
            self.new_tab_in(gid, window, cx);
        }
    }

    fn new_tab_in(&mut self, gid: GroupId, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self
            .groups
            .get(&gid)
            .and_then(|g| g.active_pane())
            .and_then(|p| self.panes.get(&p))
            .and_then(|p| p.cwd.clone());
        let id = self.alloc_pane(cwd);
        if let Some(group) = self.groups.get_mut(&gid) {
            let at = (group.active + 1).min(group.tabs.len());
            group.tabs.insert(at, id);
            group.active = at;
        }
        if let Some(ws) = self.workspaces.get_mut(self.active) {
            ws.focused = gid;
        }
        self.focus_active(window, cx);
        self.schedule_save(cx);
    }

    fn select_tab(
        &mut self,
        gid: GroupId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(group) = self.groups.get_mut(&gid)
            && index < group.tabs.len()
        {
            group.active = index;
        }
        if let Some(ws) = self.workspaces.get_mut(self.active) {
            ws.focused = gid;
        }
        self.focus_active(window, cx);
        self.schedule_save(cx);
    }

    fn cycle_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(gid) = self.focused_group() else {
            return;
        };
        let Some(group) = self.groups.get(&gid) else {
            return;
        };
        let n = group.tabs.len() as isize;
        if n > 0 {
            let next = (group.active as isize + delta).rem_euclid(n) as usize;
            self.select_tab(gid, next, window, cx);
        }
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(1, window, cx);
    }

    fn prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_tab(-1, window, cx);
    }

    /// フォーカスしている区画を分割し、新しい区画（タブ 1 つ）を右 or 下に作る
    fn split(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.focused_group() else {
            return;
        };
        let cwd = self.focused_cwd();
        let gid = self.alloc_group(cwd);
        if let Some(ws) = self.workspaces.get_mut(self.active)
            && ws.root.split(target, gid, axis)
        {
            ws.focused = gid;
        }
        self.focus_active(window, cx);
        self.schedule_save(cx);
    }

    /// Ctrl+Shift+P: コマンドパレット（コマンドを名前で検索して実行）
    fn command_palette(&mut self, _: &CommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            return;
        }
        let keys = crate::keymap::resolve(&self.settings.keybindings);
        let items = keys
            .keys
            .iter()
            .filter(|(c, _)| c.id != "command_palette")
            .map(|(c, k)| crate::picker::CommandItem {
                id: c.id.to_string(),
                label: c.label.to_string(),
                keys: crate::keymap::display_keys(k),
            })
            .collect();
        self.open_picker(
            PickerKind::Commands(items),
            String::new(),
            PickerTarget::Command,
            window,
            cx,
        );
    }

    /// コマンド id のコマンドを実行する（コマンドパレットから）
    fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = crate::keymap::action_name(id) else {
            return;
        };
        match cx.build_action(name, None) {
            Ok(action) => window.dispatch_action(action, cx),
            Err(e) => log::warn!("コマンド {id} を実行できませんでした: {e}"),
        }
    }

    /// ブラウザで押された wimux のキー（Ctrl+T など）。ブラウザは別ウィンドウなので、
    /// キーは GPUI に届かない。ブラウザの区画をアクティブにしてから、そのコマンドを実行する
    fn on_browser_key(
        &mut self,
        id: PaneId,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = crate::keymap::forwarded_action(key) else {
            return;
        };
        self.on_browser_msg(id, BrowserMsg::Focused, cx);
        // キーボードの入力先を wimux に戻す（コマンドのあとターミナルや入力欄に打てるように）
        window.activate_window();
        self.focus_handle.focus(window, cx);
        match cx.build_action(name, None) {
            Ok(action) => window.dispatch_action(action, cx),
            Err(e) => log::warn!("{key} のコマンドを実行できませんでした: {e}"),
        }
        cx.notify();
    }

    /// Ctrl+Shift+L: 右に分割して、新しい区画を空のブラウザにする（URL 欄にカーソル）。
    /// ブラウザのタブを見ているときは、分割せずにその URL 欄へ（ブラウザの Ctrl+L と同じ使い方）
    fn split_browser(&mut self, _: &SplitBrowser, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.focused_pane()
            && self.panes.get(&id).is_some_and(|p| p.is_browser())
        {
            self.focus_url_field(id, window, cx);
            return;
        }
        let Some(target) = self.focused_group() else {
            return;
        };
        let pane = self.alloc_id();
        self.panes.insert(pane, Pane::new_browser(String::new()));
        let gid = self.alloc_id();
        self.groups.insert(gid, Group::new(pane));
        if let Some(ws) = self.workspaces.get_mut(self.active)
            && ws.root.split(target, gid, Axis::Horizontal)
        {
            ws.focused = gid;
        }
        self.focus_handle.focus(window, cx);
        self.focus_url_field(pane, window, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    fn split_right(&mut self, _: &SplitRight, window: &mut Window, cx: &mut Context<Self>) {
        self.split(Axis::Horizontal, window, cx);
    }

    fn split_down(&mut self, _: &SplitDown, window: &mut Window, cx: &mut Context<Self>) {
        self.split(Axis::Vertical, window, cx);
    }

    fn close_pane(&mut self, _: &ClosePane, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.focused_pane() {
            self.close_pane_id(id, window, cx);
        }
    }

    /// タブ（ターミナル）を閉じる。区画・ワークスペースが空になったらそれも閉じる
    fn close_pane_id(&mut self, id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some((wi, gid, _)) = self.locate(id) else {
            return;
        };
        self.panes.remove(&id);
        let empty = self.groups.get_mut(&gid).is_some_and(|g| g.remove(id));
        if empty {
            self.groups.remove(&gid);
            let ws = &mut self.workspaces[wi];
            let root = std::mem::replace(&mut ws.root, Node::Leaf(0));
            match root.remove(gid) {
                Some(root) => {
                    if ws.focused == gid {
                        ws.focused = root.leaves()[0];
                    }
                    ws.root = root;
                }
                None => {
                    self.close_workspace(wi, window, cx);
                    return;
                }
            }
        }
        self.focus_active(window, cx);
        self.schedule_save(cx);
    }

    /// ワークスペースの番号（その区画を含むもの）
    fn ws_of_group(&self, gid: GroupId) -> Option<usize> {
        self.workspaces
            .iter()
            .position(|ws| ws.groups().contains(&gid))
    }

    /// タブをドラッグで移動する（並べ替え・別の区画へ・分割して置く・別のワークスペースへ）
    fn move_tab(
        &mut self,
        pane: PaneId,
        dest: TabDest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drop_hint = None;
        let Some((src_wi, src_gid, src_idx)) = self.locate(pane) else {
            return;
        };
        let src_len = self.groups.get(&src_gid).map_or(0, |g| g.tabs.len());
        match dest {
            // 自分の区画の中央、または 1 枚しかないタブで自分の区画を分割 → 何もしない（選ぶだけ）
            TabDest::Zone { gid, zone }
                if gid == src_gid && (zone == DropZone::Center || src_len == 1) =>
            {
                self.select_tab(src_gid, src_idx, window, cx);
                return;
            }
            TabDest::Workspace(wi) if wi == src_wi => return,
            // 同じ区画の中での並べ替え: 落とした先のタブの位置に入る
            TabDest::Tab { gid, index } if gid == src_gid => {
                if let Some(g) = self.groups.get_mut(&gid) {
                    g.tabs.remove(src_idx);
                    let i = index.min(g.tabs.len());
                    g.tabs.insert(i, pane);
                    g.active = i;
                }
                self.select_tab(gid, index.min(src_len - 1), window, cx);
                return;
            }
            _ => {}
        }

        // 元の区画から外す（空になった区画はツリーからも外す）
        let mut src_ws_empty = false;
        if self
            .groups
            .get_mut(&src_gid)
            .is_some_and(|g| g.remove(pane))
        {
            self.groups.remove(&src_gid);
            let ws = &mut self.workspaces[src_wi];
            let root = std::mem::replace(&mut ws.root, Node::Leaf(0));
            match root.remove(src_gid) {
                Some(root) => {
                    if ws.focused == src_gid {
                        ws.focused = root.leaves()[0];
                    }
                    ws.root = root;
                }
                None => src_ws_empty = true,
            }
        }

        // 移動先に入れる
        let placed = match dest {
            TabDest::Tab { gid, index } => self.groups.get_mut(&gid).map(|g| {
                let i = index.min(g.tabs.len());
                g.tabs.insert(i, pane);
                g.active = i;
                gid
            }),
            TabDest::Zone {
                gid,
                zone: DropZone::Center,
            } => self.groups.get_mut(&gid).map(|g| {
                g.tabs.push(pane);
                g.active = g.tabs.len() - 1;
                gid
            }),
            TabDest::Zone { gid, zone } => {
                let (axis, before) = match zone {
                    DropZone::Left => (Axis::Horizontal, true),
                    DropZone::Right => (Axis::Horizontal, false),
                    DropZone::Top => (Axis::Vertical, true),
                    _ => (Axis::Vertical, false),
                };
                let new = self.alloc_id();
                self.groups.insert(new, Group::new(pane));
                match self.ws_of_group(gid) {
                    Some(wi) if self.workspaces[wi].root.split_at(gid, new, axis, before) => {
                        Some(new)
                    }
                    _ => {
                        self.groups.remove(&new);
                        None
                    }
                }
            }
            TabDest::NewWorkspace => None,
            TabDest::Workspace(wi) => {
                let gid = self.workspaces.get(wi).map(|ws| ws.focused);
                gid.and_then(|gid| {
                    let g = self.groups.get_mut(&gid)?;
                    g.tabs.push(pane);
                    g.active = g.tabs.len() - 1;
                    Some(gid)
                })
            }
        };
        // 新しいワークスペースへ（入れられなかったときも、失わないようにここへ）
        let dest_gid = placed.unwrap_or_else(|| {
            let gid = self.alloc_id();
            self.groups.insert(gid, Group::new(pane));
            self.workspaces.push(Workspace::new(gid));
            gid
        });

        if src_ws_empty {
            self.workspaces.remove(src_wi);
        }
        if let Some(wi) = self.ws_of_group(dest_gid) {
            self.active = wi;
            self.workspaces[wi].focused = dest_gid;
        }
        self.focus_active(window, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    /// タブをドラッグしている間、区画に重ねる「ここに落とすとこうなる」表示と受け口
    fn render_drop_overlay(&self, gid: GroupId, cx: &mut Context<Self>) -> impl IntoElement {
        let hint = self.drop_hint.filter(|(g, _)| *g == gid).map(|(_, z)| z);
        let accent = th().accent;
        div()
            .id(("drop-zone", gid))
            .absolute()
            .inset_0()
            .on_drag_move::<DraggedTab>(cx.listener(
                move |this, ev: &DragMoveEvent<DraggedTab>, _, cx| {
                    let b = ev.bounds;
                    let p = ev.event.position;
                    let new = if b.contains(&p) {
                        let x = f32::from(p.x - b.origin.x) / f32::from(b.size.width).max(1.);
                        let y = f32::from(p.y - b.origin.y) / f32::from(b.size.height).max(1.);
                        Some((gid, DropZone::at(x, y)))
                    } else if this.drop_hint.is_some_and(|(g, _)| g == gid) {
                        None
                    } else {
                        return;
                    };
                    if this.drop_hint != new {
                        this.drop_hint = new;
                        cx.notify();
                    }
                },
            ))
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                let zone = this
                    .drop_hint
                    .filter(|(g, _)| *g == gid)
                    .map_or(DropZone::Center, |(_, z)| z);
                this.move_tab(d.pane, TabDest::Zone { gid, zone }, window, cx);
            }))
            .when_some(hint, |el, zone| {
                let preview = div()
                    .absolute()
                    .bg(rgba((accent << 8) | 0x30))
                    .border_2()
                    .border_color(rgb(accent))
                    .rounded(px(4.));
                let preview = match zone {
                    DropZone::Center => preview.inset(px(4.)),
                    DropZone::Left => preview.top_0().bottom_0().left_0().w(relative(0.5)),
                    DropZone::Right => preview.top_0().bottom_0().right_0().w(relative(0.5)),
                    DropZone::Top => preview.left_0().right_0().top_0().h(relative(0.5)),
                    DropZone::Bottom => preview.left_0().right_0().bottom_0().h(relative(0.5)),
                };
                el.child(preview)
            })
    }

    fn move_focus(&mut self, dir: Direction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ws) = self.workspaces.get_mut(self.active) else {
            return;
        };
        if let Some(next) = ws.root.neighbor(ws.focused, dir) {
            ws.focused = next;
            self.focus_active(window, cx);
        }
    }

    fn focus_left(&mut self, _: &FocusLeft, w: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(Direction::Left, w, cx);
    }
    fn focus_right(&mut self, _: &FocusRight, w: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(Direction::Right, w, cx);
    }
    fn focus_up(&mut self, _: &FocusUp, w: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(Direction::Up, w, cx);
    }
    fn focus_down(&mut self, _: &FocusDown, w: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(Direction::Down, w, cx);
    }

    /// Alt+Shift+矢印: フォーカス中の区画の境界をその方向へ動かす
    fn resize_focused(&mut self, dir: Direction, cx: &mut Context<Self>) {
        let Some(ws) = self.workspaces.get_mut(self.active) else {
            return;
        };
        if ws.root.nudge(ws.focused, dir, 0.05) {
            self.schedule_save(cx);
            cx.notify();
        }
    }

    fn resize_left(&mut self, _: &ResizeLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.resize_focused(Direction::Left, cx);
    }
    fn resize_right(&mut self, _: &ResizeRight, _: &mut Window, cx: &mut Context<Self>) {
        self.resize_focused(Direction::Right, cx);
    }
    fn resize_up(&mut self, _: &ResizeUp, _: &mut Window, cx: &mut Context<Self>) {
        self.resize_focused(Direction::Up, cx);
    }
    fn resize_down(&mut self, _: &ResizeDown, _: &mut Window, cx: &mut Context<Self>) {
        self.resize_focused(Direction::Down, cx);
    }

    /// 分割の境界のドラッグ中: マウスの位置から比率を決める
    fn on_divider_drag(
        &mut self,
        path: &[bool],
        axis: Axis,
        ev: &DragMoveEvent<DraggedDivider>,
        cx: &mut Context<Self>,
    ) {
        // どの分割の容器にも届くので、つかんだ境界の分割だけを動かす
        if ev.drag(cx).path != path {
            return;
        }
        let b = ev.bounds;
        let pos = ev.event.position;
        let ratio = match axis {
            Axis::Horizontal => (pos.x - b.origin.x) / b.size.width,
            Axis::Vertical => (pos.y - b.origin.y) / b.size.height,
        };
        if !ratio.is_finite() {
            return;
        }
        let Some(ws) = self.workspaces.get_mut(self.active) else {
            return;
        };
        if ws.root.set_ratio_at(path, ratio) {
            self.schedule_save(cx);
            cx.notify();
        }
    }

    /// タブをドラッグ中か（境界のドラッグは除く）
    fn tab_dragging(&self, cx: &App) -> bool {
        cx.has_active_drag() && !self.resizing
    }

    /// Ctrl+Shift+G: ターミナルで `cdi` を実行する（フォルダ履歴をターミナル内で選ぶ）
    fn open_dir_picker(&mut self, _: &OpenDirPicker, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self
            .focused_pane()
            .and_then(|id| self.panes.get(&id))
            .and_then(|p| p.view.clone())
        {
            view.update(cx, |v, _| v.send_text("cdi\r"));
        }
    }

    fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.font_size = size.clamp(8., 32.);
        let size = self.font_size;
        for pane in self.panes.values() {
            if let Some(view) = &pane.view {
                view.update(cx, |v, cx| {
                    v.set_font_size(size);
                    cx.notify();
                });
            }
        }
        if self.settings.font_size != size {
            self.settings.font_size = size;
            settings::save(&self.settings);
        }
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // ブラウザ
    // -----------------------------------------------------------------------

    /// Ctrl+Shift+O: フォーカス中の区画に空のブラウザのタブを開き、URL 欄にカーソルを置く
    fn open_browser(&mut self, _: &OpenBrowser, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(gid) = self.focused_group() {
            self.open_browser_in(gid, String::new(), window, cx);
        }
    }

    /// 区画に新しいブラウザのタブを作る。URL が空なら URL 欄にカーソルを置く
    fn open_browser_in(
        &mut self,
        gid: GroupId,
        url: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.alloc_id();
        let blank = url.is_empty();
        self.panes.insert(id, Pane::new_browser(url));
        if let Some(group) = self.groups.get_mut(&gid) {
            let at = (group.active + 1).min(group.tabs.len());
            group.tabs.insert(at, id);
            group.active = at;
        }
        if let Some(ws) = self.workspaces.get_mut(self.active) {
            ws.focused = gid;
        }
        self.focus_handle.focus(window, cx);
        if blank {
            self.focus_url_field(id, window, cx);
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// シェルの `web [URL]`: そのターミナルと同じ区画に新しいブラウザのタブを開く
    fn open_web_from(
        &mut self,
        id: PaneId,
        arg: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((wi, gid, _)) = self.locate(id) else {
            return;
        };
        if wi != self.active {
            return;
        }
        let url = if arg.is_empty() {
            String::new()
        } else {
            browser::normalize_url(&arg)
        };
        self.open_browser_in(gid, url, window, cx);
    }

    /// ブラウザのページが新しいウィンドウを開こうとした（右クリックの「新しいウィンドウで開く」など）:
    /// 別ウィンドウにはせず、そのブラウザと同じ区画の新しいタブで開く
    fn open_new_window_from(
        &mut self,
        id: PaneId,
        url: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((wi, gid, _)) = self.locate(id) else {
            return;
        };
        if wi != self.active || url.is_empty() {
            return;
        }
        self.open_browser_in(gid, url, window, cx);
    }

    /// URL 欄（入力欄）を（まだなら）作る
    fn ensure_url_field(
        &mut self,
        id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<TextField>> {
        let pane = self.panes.get(&id)?;
        if !pane.is_browser() {
            return None;
        }
        if let Some(f) = &pane.url_field {
            return Some(f.clone());
        }
        let url = pane.url.clone().unwrap_or_default();
        let field = cx.new(|cx| {
            let mut f = TextField::new(
                "URL・ポート番号（3000 → localhost:3000）・検索語を入力して Enter",
                cx,
            );
            f.set_text(&url);
            f
        });
        let sub = cx.subscribe_in(
            &field,
            window,
            move |this, field, ev: &TextFieldEvent, window, cx| {
                match ev {
                    TextFieldEvent::Submit(text) if !text.is_empty() => {
                        let url = browser::normalize_url(text);
                        field.update(cx, |f, _| f.set_text(&url));
                        this.navigate(id, &url, cx);
                        this.focus_handle.focus(window, cx);
                    }
                    TextFieldEvent::Submit(_)
                    | TextFieldEvent::SubmitBack
                    | TextFieldEvent::Changed => {}
                    TextFieldEvent::Cancel => {
                        let url = this
                            .panes
                            .get(&id)
                            .and_then(|p| p.url.clone())
                            .unwrap_or_default();
                        field.update(cx, |f, _| f.set_text(&url));
                        this.focus_handle.focus(window, cx);
                    }
                }
                cx.notify();
            },
        );
        let pane = self.panes.get_mut(&id)?;
        pane.url_field = Some(field.clone());
        pane._field_sub = Some(sub);
        Some(field)
    }

    fn focus_url_field(&mut self, id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(field) = self.ensure_url_field(id, window, cx) {
            field.update(cx, |f, cx| f.focus(window, cx));
            // まだ描画されていない入力欄はフォーカスが定着しないので、描画後にもう一度
            cx.on_next_frame(window, move |_, window, cx| {
                field.update(cx, |f, cx| {
                    if !f.focus_handle(cx).is_focused(window) {
                        f.focus(window, cx);
                    }
                });
            });
        }
    }

    fn navigate(&mut self, id: PaneId, url: &str, cx: &mut Context<Self>) {
        if let Some(pane) = self.panes.get_mut(&id) {
            pane.url = Some(url.to_string());
            pane.title = None;
            if let Some(b) = &pane.browser {
                b.load(url);
            }
        }
        self.schedule_save(cx);
        cx.notify();
    }

    fn on_browser_msg(&mut self, id: PaneId, msg: BrowserMsg, cx: &mut Context<Self>) {
        let Some(pane) = self.panes.get_mut(&id) else {
            return;
        };
        match msg {
            BrowserMsg::Title(t) => pane.title = Some(t),
            BrowserMsg::Loading(l) => pane.loading = l,
            // 新しいタブを開く・キーを回すのは上（受信側）で処理する
            BrowserMsg::NewWindow(_) | BrowserMsg::Key(_) => {}
            // ブラウザをクリックした: その区画・タブをアクティブにする（タブの青い線を移す）
            BrowserMsg::Focused => {
                pane.attention = false;
                if let Some((wi, gid, ti)) = self.locate(id)
                    && wi == self.active
                {
                    self.workspaces[wi].focused = gid;
                    if let Some(group) = self.groups.get_mut(&gid) {
                        group.active = ti;
                    }
                    self.schedule_save(cx);
                }
            }
            BrowserMsg::Url(u) => {
                if u.is_empty() || u == "about:blank" {
                    return;
                }
                // URL 欄は、ユーザーが打ち込み中でなければ常に今のページの URL にそろえる
                if let Some(field) = &pane.url_field {
                    let new = u.clone();
                    field.update(cx, |f, cx| {
                        if !f.is_edited() && f.text() != new {
                            f.set_text(&new);
                            cx.notify();
                        }
                    });
                }
                if pane.url.as_deref() != Some(u.as_str()) {
                    pane.url = Some(u);
                    self.schedule_save(cx);
                }
            }
        }
    }

    /// ポップアップを開いている間はブラウザを隠す（子ウィンドウなので上に描けないため）
    fn overlay_open(&self) -> bool {
        self.picker.is_some() || self.settings_open || self.ws_menu.is_some()
    }

    /// 表示する区画のブラウザを（まだなら）作り、見えないブラウザは隠す
    fn sync_browsers(&mut self, visible: &[PaneId], window: &mut Window, cx: &mut Context<Self>) {
        // ポップアップを開いている間・タブをドラッグしている間はブラウザを隠す
        // （ブラウザは別ウィンドウなので、上に予告を描けず、落とすこともできないため）
        // 境界のドラッグ（区画の大きさの変更）中は隠さない。隠すと描画のたびに
        // 「隠す → 表示」を繰り返してチカチカする
        let show: Vec<PaneId> = if self.overlay_open() || self.tab_dragging(cx) {
            Vec::new()
        } else {
            visible.to_vec()
        };
        // 表示するブラウザのタブには URL 欄を用意する（前回から復元したタブにも）
        for id in visible {
            self.ensure_url_field(*id, window, cx);
        }
        for id in &show {
            let Some(pane) = self.panes.get_mut(id) else {
                continue;
            };
            if pane.browser.is_some()
                || pane.browser_error.is_some()
                || pane.browser_pending
                || !pane.is_browser()
            {
                continue;
            }
            if pane.url.as_deref().is_none_or(str::is_empty) {
                // URL が未入力のタブはまだブラウザを作らない
                continue;
            }
            if !browser::SUPPORTED {
                pane.browser_error =
                    Some("この OS ではブラウザペインは使えません（Windows 専用）".into());
                continue;
            }
            pane.browser_pending = true;
            let url = pane.url.clone().unwrap_or_default();
            let id = *id;
            let parent = match wry::raw_window_handle::HasWindowHandle::window_handle(&*window) {
                Ok(h) => browser::ParentWindow(h.as_raw()),
                Err(e) => {
                    pane.browser_error = Some(e.to_string());
                    continue;
                }
            };
            let context = self
                .web_context
                .get_or_insert_with(|| {
                    Rc::new(std::cell::RefCell::new(wry::WebContext::new(Some(
                        browser::data_dir(),
                    ))))
                })
                .clone();
            let tx = self.browser_tx.clone();
            // WebView2 の作成中は Windows のメッセージが処理される。GPUI の処理の途中で作ると
            // 二重に呼び出されて壊れるため、GPUI の処理の外（非同期タスク）で作る
            cx.spawn(async move |this, cx| {
                let result = {
                    let mut ctx = context.borrow_mut();
                    Browser::new(parent, &mut ctx, id, &url, tx)
                };
                let _ = this.update(cx, |this, cx| {
                    if let Some(pane) = this.panes.get_mut(&id) {
                        pane.browser_pending = false;
                        match result {
                            Ok(b) => pane.browser = Some(Rc::new(b)),
                            Err(e) => {
                                log::error!("ブラウザを開けませんでした: {e}");
                                pane.browser_error = Some(e.to_string());
                            }
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        for (id, pane) in &self.panes {
            if let Some(b) = &pane.browser
                && !show.contains(id)
            {
                b.set_visible(false);
            }
        }
    }

    // -----------------------------------------------------------------------
    // 設定
    // -----------------------------------------------------------------------

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = true;
        self.ws_menu = None;
        self.settings_focus.focus(window, cx);
        cx.notify();
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.focus_active(window, cx);
    }

    fn set_theme(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        theme::apply(name);
        self.settings.theme = name.to_string();
        settings::save(&self.settings);
        // ターミナルも含めて描き直す
        for pane in self.panes.values() {
            if let Some(view) = &pane.view {
                view.update(cx, |_, cx| cx.notify());
            }
        }
        window.refresh();
        cx.notify();
    }

    /// ターミナルを含めて描き直す
    fn refresh_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for pane in self.panes.values() {
            if let Some(view) = &pane.view {
                view.update(cx, |_, cx| cx.notify());
            }
        }
        window.refresh();
        cx.notify();
    }

    fn set_opacity(&mut self, v: f32, window: &mut Window, cx: &mut Context<Self>) {
        theme::set_opacity(v);
        self.settings.opacity = theme::opacity();
        apply_backdrop(&self.settings, window);
        settings::save(&self.settings);
        self.refresh_all(window, cx);
    }

    fn set_backdrop(&mut self, b: Backdrop, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.backdrop = b;
        apply_backdrop(&self.settings, window);
        settings::save(&self.settings);
        self.refresh_all(window, cx);
    }

    fn set_line_height(&mut self, v: f32, window: &mut Window, cx: &mut Context<Self>) {
        theme::set_line_height(v);
        self.settings.line_height = theme::line_height();
        settings::save(&self.settings);
        self.refresh_all(window, cx);
    }

    fn set_cursor(&mut self, kind: CursorKind, window: &mut Window, cx: &mut Context<Self>) {
        crate::terminal::set_default_cursor(kind);
        self.settings.cursor = kind;
        settings::save(&self.settings);
        for pane in self.panes.values() {
            if let Some(view) = &pane.view {
                view.update(cx, |v, _| v.apply_config());
            }
        }
        self.refresh_all(window, cx);
    }

    fn set_font_family(&mut self, family: Option<String>, cx: &mut Context<Self>) {
        self.settings.font_family = family.clone();
        settings::save(&self.settings);
        for pane in self.panes.values() {
            if let Some(view) = &pane.view {
                view.update(cx, |v, cx| {
                    v.set_font_family(family.as_deref(), cx);
                    cx.notify();
                });
            }
        }
        cx.notify();
    }

    fn zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.set_font_size(self.font_size + 1., cx);
    }
    fn zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.set_font_size(self.font_size - 1., cx);
    }
    fn zoom_reset(&mut self, _: &ZoomReset, _: &mut Window, cx: &mut Context<Self>) {
        self.set_font_size(DEFAULT_FONT_SIZE, cx);
    }
    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // ポップアップ
    // -----------------------------------------------------------------------

    fn open_picker(
        &mut self,
        kind: PickerKind,
        initial: String,
        target: PickerTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let history = &self.history;
        let picker = cx.new(|cx| Picker::new(kind, initial, Some(history), cx));
        let sub = cx.subscribe_in(&picker, window, |this, _, ev: &PickerEvent, window, cx| {
            this.on_picker_event(ev, window, cx);
        });
        picker.read(cx).focus_handle(cx).focus(window, cx);
        self.picker = Some((picker, target, sub));
        cx.notify();
    }

    fn on_picker_event(&mut self, ev: &PickerEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            PickerEvent::QueryChanged => {
                if let Some((picker, _, _)) = &self.picker {
                    let history = &self.history;
                    picker.update(cx, |p, cx| {
                        p.refresh(Some(history));
                        cx.notify();
                    });
                }
            }
            PickerEvent::Dismissed => {
                self.picker = None;
                self.focus_active(window, cx);
            }
            PickerEvent::Confirmed(value) => {
                let Some((_, target, _)) = self.picker.take() else {
                    return;
                };
                match target {
                    PickerTarget::Rename(index) => {
                        if let Some(ws) = self.workspaces.get_mut(index) {
                            ws.custom_name = (!value.is_empty()).then(|| value.clone());
                        }
                        self.schedule_save(cx);
                    }
                    PickerTarget::Command => {
                        self.focus_active(window, cx);
                        self.run_command(value, window, cx);
                        return;
                    }
                }
                self.focus_active(window, cx);
            }
        }
    }

    // -----------------------------------------------------------------------
    // ターミナルからのイベント
    // -----------------------------------------------------------------------

    fn on_terminal_event(
        &mut self,
        id: PaneId,
        ev: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            TerminalEvent::TitleChanged(title) => {
                let Some(pane) = self.panes.get_mut(&id) else {
                    return;
                };
                pane.title = Some(title.clone());
                let Some(new_state) = claude_state_from_title(title) else {
                    // Claude のタイトルでなくなった = タイトルで見ていた Claude は終わった
                    if pane.agent == Some(CLAUDE) && !pane.hooked && !pane.activity_driven {
                        pane.agent = None;
                        pane.state = AgentState::None;
                    }
                    cx.notify();
                    return;
                };
                pane.agent = Some(CLAUDE);
                pane.activity_driven = false;
                if pane.hooked {
                    // 状態はフックで分かる。フックが届かなかったときのために「作業中」だけは拾う
                    if new_state == AgentState::Working
                        && matches!(pane.state, AgentState::Idle | AgentState::None)
                    {
                        pane.state = AgentState::Working;
                        pane.working_since = Some(Instant::now());
                    } else if pane.state == AgentState::None {
                        pane.state = new_state;
                    }
                    cx.notify();
                    return;
                }
                // 許可待ちで止まっている間は、タイトルが待機中になっても「入力待ち」のまま
                if pane.state == AgentState::Asking && new_state == AgentState::Idle {
                    return;
                }
                let finished = pane.state == AgentState::Working && new_state == AgentState::Idle;
                pane.state = new_state;
                if finished {
                    // 許可の確認で止まったときもタイトルは待機中になる。直後にベルが来たら
                    // 「入力待ち」なので、少し待ってから「完了」を通知する
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(700))
                            .await;
                        let _ = this.update_in(cx, |this, window, cx| {
                            if this
                                .panes
                                .get(&id)
                                .is_some_and(|p| p.state == AgentState::Idle && !p.hooked)
                            {
                                this.notify_pane(id, "Claude Code", "応答が終わりました", window);
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }
            }
            TerminalEvent::Shell(ShellEvent::Cwd(cwd)) => {
                let Some(pane) = self.panes.get_mut(&id) else {
                    return;
                };
                // プロンプトが出た = Claude Code などのアプリは終了している
                pane.state = AgentState::None;
                pane.agent = None;
                pane.hooked = false;
                pane.activity_driven = false;
                // git checkout でブランチが変わることもあるので、プロンプトのたびに読み直す
                pane.branch = git_branch(cwd);
                if pane.cwd.as_deref() != Some(cwd.as_str()) || !pane.prompted {
                    pane.prompted = true;
                    pane.cwd = Some(cwd.clone());
                    self.history.record(cwd);
                    self.schedule_save(cx);
                }
            }
            TerminalEvent::Shell(ShellEvent::UserVar { name, value }) => match name.as_str() {
                "WIMUX_CONDA" => {
                    if let Some(pane) = self.panes.get_mut(&id) {
                        let conda = (!value.is_empty()).then(|| value.clone());
                        if pane.conda != conda {
                            pane.conda = conda;
                            self.schedule_save(cx);
                        }
                    }
                }
                // シェル連携のラッパー（codex・gemini など）がエージェントを起動する直前に送る
                "WIMUX_AGENT" => {
                    if let (Some(label), Some(pane)) = (agent_label(value), self.panes.get_mut(&id))
                    {
                        pane.agent = Some(label);
                        // Claude はフック・タイトルで分かる。ほかは出力の動きで推定する
                        pane.activity_driven = label != CLAUDE;
                        if pane.state == AgentState::None {
                            pane.state = AgentState::Idle;
                        }
                    }
                }
                // シェル連携スクリプトのコマンド（web）から呼ばれる
                "WIMUX_CMD" => {
                    let mut parts = value.splitn(2, ' ');
                    let cmd = parts.next().unwrap_or("");
                    let arg = parts.next().unwrap_or("").trim().to_string();
                    match cmd {
                        // `web [url]`: 同じ区画に新しいブラウザのタブを開く
                        "web" => self.open_web_from(id, arg, window, cx),
                        _ => {}
                    }
                }
                _ => {}
            },
            TerminalEvent::Shell(ShellEvent::Notify { title, body }) => {
                let (agent_active, hooked) = self
                    .panes
                    .get(&id)
                    .map_or((false, false), |p| (p.state != AgentState::None, p.hooked));
                // エージェントの「許可が必要」などの通知 → 入力待ち（フックがあればフックに任せる）
                if agent_active && !hooked && !body.contains("waiting for your input") {
                    self.claude_asking(id, body, window);
                } else {
                    let title = title.clone().unwrap_or_else(|| "通知".into());
                    self.notify_pane(id, &title, body, window);
                }
            }
            // Claude Code は許可・質問で止まったときにベルを鳴らす（通知方法が「ベル」のとき）
            TerminalEvent::Bell => {
                if self
                    .panes
                    .get(&id)
                    .is_some_and(|p| p.state != AgentState::None && !p.hooked)
                {
                    self.claude_asking(id, "許可・回答を待っています", window);
                }
            }
            TerminalEvent::OpenUrl { url, in_app } => {
                if *in_app && browser::SUPPORTED {
                    self.open_web_from(id, url.clone(), window, cx);
                } else {
                    crate::open::open_url(url);
                }
            }
            TerminalEvent::OpenPath { path, line, col } => {
                crate::open::open_path(path, *line, *col, self.settings.editor.as_deref());
            }
            TerminalEvent::Exited => {
                if !self.closing {
                    self.close_pane_id(id, window, cx);
                }
            }
            TerminalEvent::Focused => {
                if let Some(pane) = self.panes.get_mut(&id) {
                    pane.attention = false;
                }
                if let Some((wi, gid, ti)) = self.locate(id)
                    && wi == self.active
                {
                    self.workspaces[wi].focused = gid;
                    if let Some(group) = self.groups.get_mut(&gid) {
                        group.active = ti;
                    }
                }
            }
        }
        cx.notify();
    }

    /// Claude Code が許可・質問への回答を待って止まった
    fn claude_asking(&mut self, id: PaneId, body: &str, window: &Window) {
        let Some(pane) = self.panes.get_mut(&id) else {
            return;
        };
        let was_asking = pane.state == AgentState::Asking;
        pane.state = AgentState::Asking;
        if !was_asking {
            pane.asking_since = Some(Instant::now());
            // 直前の「完了」通知と重なっても、こちらは必ず知らせる
            pane.last_notified = None;
            let title = format!("{}（入力待ち）", agent_title(pane.agent));
            self.notify_pane(id, &title, body, window);
        }
    }

    // -----------------------------------------------------------------------
    // エージェントの状態（フック・出力の動き）と入力待ちへの移動
    // -----------------------------------------------------------------------

    /// ペインの中のフック・`sondev notify` から届いた知らせ
    fn on_ipc(&mut self, msg: crate::ipc::Incoming, window: &mut Window, cx: &mut Context<Self>) {
        use crate::ipc::{AgentEvent, Message};
        // WIMUX_PANE_ID は「<プロセス ID>-<ペイン番号>」。別に起動した wimux のものは無視する
        let Some(id) = msg
            .pane
            .split_once('-')
            .filter(|(pid, _)| *pid == std::process::id().to_string())
            .and_then(|(_, id)| id.parse::<PaneId>().ok())
            .filter(|id| self.panes.contains_key(id))
        else {
            return;
        };
        match msg.message {
            Message::Usage {
                agent,
                five_hour,
                seven_day,
            } => {
                let label = agent.as_deref().and_then(agent_label).unwrap_or(CLAUDE);
                let i = match self.usage.iter().position(|(l, _)| *l == label) {
                    Some(i) => i,
                    None => {
                        self.usage.push((label, AgentUsage::default()));
                        self.usage.len() - 1
                    }
                };
                let usage = &mut self.usage[i].1;
                // 届いた枠だけ新しくする（無い枠は戻る時刻を過ぎたら表示から消える）
                if five_hour.is_some() {
                    usage.five_hour = five_hour;
                }
                if seven_day.is_some() {
                    usage.seven_day = seven_day;
                }
            }
            Message::Notify { title, body } => {
                let title = title.unwrap_or_else(|| "通知".into());
                self.notify_pane(id, &title, &body, window);
            }
            Message::Agent {
                agent,
                event,
                message,
                session,
            } => {
                let label = agent_label(&agent).unwrap_or("Agent");
                let Some(pane) = self.panes.get_mut(&id) else {
                    return;
                };
                pane.agent = Some(label);
                if let Some(s) = session.filter(|s| crate::claude_session::valid_id(s)) {
                    pane.agent_session = Some(s);
                }
                // Claude Code はフックで状態がすべて分かる。Codex の notify は「完了」だけ
                if label == CLAUDE {
                    pane.hooked = true;
                    pane.activity_driven = false;
                }
                match event {
                    AgentEvent::Start => {
                        if pane.state == AgentState::None {
                            pane.state = AgentState::Idle;
                        }
                    }
                    AgentEvent::Working => {
                        if pane.state != AgentState::Working {
                            pane.working_since = Some(Instant::now());
                        }
                        pane.state = AgentState::Working;
                    }
                    AgentEvent::Asking => {
                        let body = message.unwrap_or_else(|| "許可・回答を待っています".into());
                        self.claude_asking(id, &body, window);
                    }
                    AgentEvent::Done => {
                        pane.state = AgentState::Idle;
                        let title = agent_title(pane.agent);
                        let body = message.unwrap_or_else(|| "応答が終わりました".into());
                        self.notify_pane(id, title, &body, window);
                    }
                    AgentEvent::IdleReminder => {}
                    AgentEvent::End => {
                        pane.state = AgentState::None;
                        pane.agent = None;
                        pane.hooked = false;
                    }
                }
            }
        }
        cx.notify();
    }

    /// 0.5 秒ごと: 入力待ちに答えたか、フックのないエージェントが動いているかを見る
    fn tick_agents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let quiet = |t: Option<Instant>, d: Duration| t.is_none_or(|t| now.duration_since(t) >= d);
        let mut changed = false;
        let mut finished = Vec::new();
        for (id, pane) in self.panes.iter_mut() {
            if pane.agent.is_none() {
                continue;
            }
            let Some(view) = &pane.view else {
                continue;
            };
            let a = view.read(cx).activity();
            // 入力待ちのあとにキーを押した = 答えた → 作業中に戻す
            if pane.state == AgentState::Asking
                && let (Some(input), Some(asked)) = (a.last_input, pane.asking_since)
                && input > asked
            {
                pane.state = AgentState::Working;
                pane.working_since = Some(now);
                changed = true;
                continue;
            }
            if !pane.activity_driven || pane.hooked {
                continue;
            }
            // 自分で打った文字の表示や、リサイズの再描画は「動いている」に数えない
            let busy = !quiet(a.last_output, AGENT_QUIET)
                && quiet(a.last_input, AGENT_QUIET)
                && quiet(a.last_resize, Duration::from_secs(1));
            match pane.state {
                AgentState::Idle | AgentState::None if busy => {
                    pane.state = AgentState::Working;
                    pane.working_since = Some(now);
                    changed = true;
                }
                AgentState::Working if quiet(a.last_output, AGENT_QUIET) => {
                    pane.state = AgentState::Idle;
                    changed = true;
                    if pane
                        .working_since
                        .is_some_and(|t| now.duration_since(t) >= AGENT_MIN_WORK)
                    {
                        finished.push((*id, agent_title(pane.agent)));
                    }
                }
                _ => {}
            }
        }
        for (id, title) in finished {
            self.notify_pane(id, title, "応答が終わりました", window);
        }
        // 利用上限の「あと何分」は 1 分ごとに描き直す
        let minute = crate::hooks::now_secs() / 60;
        if self.usage.iter().any(|(_, u)| u.any()) && minute != self.usage_minute {
            self.usage_minute = minute;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    /// Ctrl+Shift+U: 入力待ち（許可・質問）→ 完了して未確認 → そのほかの通知 の順に、次のペインへ移動
    fn jump_to_attention(
        &mut self,
        _: &JumpToAttention,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let order: Vec<PaneId> = self
            .workspaces
            .iter()
            .flat_map(|ws| self.ws_panes(ws))
            .collect();
        let current = self.focused_pane();
        let score = |id: &PaneId| {
            self.panes
                .get(id)
                .map_or(0, |p| match (p.state, p.attention) {
                    (AgentState::Asking, _) => 3,
                    (AgentState::Idle, true) => 2,
                    (_, true) => 1,
                    _ => 0,
                })
        };
        let best = order
            .iter()
            .filter(|id| Some(**id) != current)
            .map(score)
            .max()
            .unwrap_or(0);
        if best == 0 {
            return;
        }
        // 今のペインの次から順に探す（何度も押すと同じ種類のペインを巡回する）
        let start = current
            .and_then(|c| order.iter().position(|id| *id == c))
            .map_or(0, |i| i + 1);
        let n = order.len();
        let Some(target) = (0..n)
            .map(|k| order[(start + k) % n])
            .find(|id| Some(*id) != current && score(id) == best)
        else {
            return;
        };
        self.reveal_pane(target, window, cx);
    }

    /// ペインのあるワークスペース・区画・タブを表示してフォーカスする
    fn reveal_pane(&mut self, id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some((wi, gid, ti)) = self.locate(id) else {
            return;
        };
        self.active = wi;
        if let Some(ws) = self.workspaces.get_mut(wi) {
            ws.unread = 0;
            ws.focused = gid;
        }
        if let Some(group) = self.groups.get_mut(&gid) {
            group.active = ti;
        }
        self.focus_active(window, cx);
        self.schedule_save(cx);
    }

    /// 通知: 見えていないワークスペースなら未読を増やし、ウィンドウが裏ならトーストも出す
    fn notify_pane(&mut self, id: PaneId, title: &str, body: &str, window: &Window) {
        // Claude Code 自身の通知とタイトル変化の検知が重なることがあるので、短時間の重複は捨てる
        if let Some(pane) = self.panes.get_mut(&id) {
            if pane
                .last_notified
                .is_some_and(|t| t.elapsed() < Duration::from_secs(3))
            {
                return;
            }
            pane.last_notified = Some(Instant::now());
        }
        let Some((wi, gid, ti)) = self.locate(id) else {
            return;
        };
        let window_active = window.is_window_active();
        let shown = self.groups.get(&gid).is_some_and(|g| g.active == ti);
        let visible = window_active && wi == self.active && shown;
        let focused = visible && self.workspaces[wi].focused == gid;
        if !focused && let Some(pane) = self.panes.get_mut(&id) {
            pane.attention = true;
        }
        let name = self.workspace_name(wi);
        let ws = &mut self.workspaces[wi];
        ws.last_notification = Some(format!("{title}: {body}"));
        if !visible {
            ws.unread += 1;
        }
        if !window_active {
            notify::toast(&format!("{title}（{name}）"), body);
        }
    }

    // -----------------------------------------------------------------------
    // 保存・復元
    // -----------------------------------------------------------------------

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if self.save_task.is_some() || self.closing {
            return;
        }
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(800))
                .await;
            let _ = this.update(cx, |this, _| {
                this.save_task = None;
                this.save_now();
            });
        }));
    }

    fn save_now(&self) {
        if let Err(e) = session::save_session(&session::session_path(), &self.to_session()) {
            log::warn!("セッションを保存できませんでした: {e}");
        }
        if let Err(e) = self.history.save(&session::dirs_path()) {
            log::warn!("フォルダ履歴を保存できませんでした: {e}");
        }
        // シェルの cdi が読む一覧（よく行く順・1 行 1 フォルダ）
        let list = self.history.query("", 500).join("\n");
        if let Err(e) =
            session::write_atomic(&session::config_dir().join("dirs.txt"), list.as_bytes())
        {
            log::warn!("フォルダ一覧を保存できませんでした: {e}");
        }
    }

    fn to_session(&self) -> SessionFile {
        SessionFile {
            version: session::SESSION_VERSION,
            active_workspace: self.active,
            workspaces: self
                .workspaces
                .iter()
                .map(|ws| WorkspaceState {
                    name: ws.custom_name.clone(),
                    layout: Some(self.node_to_state(&ws.root)),
                    focused: ws
                        .groups()
                        .iter()
                        .position(|g| *g == ws.focused)
                        .unwrap_or(0),
                    ..Default::default()
                })
                .collect(),
        }
    }

    fn pane_state(&self, id: PaneId) -> PaneState {
        let pane = self.panes.get(&id);
        // Claude 以外の、動いているエージェントのコマンド名
        let other_agent = pane
            .filter(|p| p.state != AgentState::None)
            .and_then(|p| agent_command(p.agent?));
        PaneState {
            cwd: pane.and_then(|p| p.cwd.clone()),
            conda: pane.and_then(|p| p.conda.clone()),
            claude: pane.is_some_and(|p| p.agent == Some(CLAUDE) && p.state != AgentState::None),
            // フックが記録した今のセッション。まだ無ければ前回から引き継いだもの
            claude_session: pane
                .filter(|p| p.agent == Some(CLAUDE) && p.state != AgentState::None)
                .and_then(|p| {
                    crate::claude_session::read(&crate::claude_session::pane_key(id))
                        .or_else(|| p.agent_session.clone())
                }),
            agent: other_agent.map(str::to_string),
            agent_session: other_agent.and(pane.and_then(|p| p.agent_session.clone())),
            url: pane.and_then(|p| p.url.clone()),
        }
    }

    fn node_to_state(&self, node: &Node) -> NodeState {
        match node {
            Node::Leaf(gid) => {
                let group = self.groups.get(gid);
                NodeState::Group {
                    active: group.map_or(0, |g| g.active),
                    tabs: group
                        .map(|g| g.tabs.iter().map(|id| self.pane_state(*id)).collect())
                        .unwrap_or_default(),
                }
            }
            Node::Split {
                axis,
                ratio,
                first,
                second,
            } => NodeState::Split {
                axis: *axis,
                ratio: *ratio,
                first: Box::new(self.node_to_state(first)),
                second: Box::new(self.node_to_state(second)),
            },
        }
    }

    fn restore(&mut self, s: SessionFile) {
        for ws_state in s.workspaces {
            let (root, focused_index) = if let Some(layout) = ws_state.layout {
                (self.node_from_state(layout), ws_state.focused)
            } else {
                // v1: 表示していたタブの分割をそのまま使い、ほかのタブのペインは
                // フォーカスしていた区画のタブとして並べる
                let mut tabs = ws_state.tabs;
                if tabs.is_empty() {
                    continue;
                }
                let active = ws_state.active_tab.min(tabs.len() - 1);
                let main = tabs.remove(active);
                let root = self.node_from_state(main.layout);
                let leaves = root.leaves();
                let target = leaves.get(main.focused).copied().unwrap_or(leaves[0]);
                for tab in tabs {
                    for pane_state in flatten_panes(tab.layout) {
                        let id = self.restore_pane(pane_state);
                        if let Some(g) = self.groups.get_mut(&target) {
                            g.tabs.push(id);
                        }
                    }
                }
                (root, main.focused)
            };
            let leaves = root.leaves();
            let focused = leaves.get(focused_index).copied().unwrap_or(leaves[0]);
            let mut ws = Workspace::new(focused);
            ws.root = root;
            ws.custom_name = ws_state.name;
            self.workspaces.push(ws);
        }
        if self.workspaces.is_empty() {
            let gid = self.alloc_group(None);
            self.workspaces.push(Workspace::new(gid));
        }
        self.active = s.active_workspace.min(self.workspaces.len() - 1);
    }

    /// 保存されていたペインを作る（起動はまだしない）
    fn restore_pane(&mut self, p: PaneState) -> PaneId {
        if let Some(url) = p.url {
            let id = self.alloc_id();
            self.panes.insert(id, Pane::new_browser(url));
            return id;
        }
        let id = self.alloc_pane(p.cwd.clone());
        let pane = self.panes.get_mut(&id).expect("just allocated");
        // conda 環境 → Claude Code の順に再開。
        // conda は使うまで初期化しない（起動を速くするため）ので、base も明示的に有効にする
        if let Some(env) = p.conda.as_ref().filter(|e| !e.is_empty()) {
            pane.restore_cmds
                .push(format!("conda activate '{}'", env.replace('\'', "''")));
        }
        if p.claude && self.settings.restore_claude && restore_claude_enabled() {
            // セッション ID が分かればそのセッションを、分からなければフォルダの最新を開く
            let session = p
                .claude_session
                .clone()
                .filter(|s| crate::claude_session::valid_id(s));
            pane.restore_cmds.push(match &session {
                Some(s) => format!("claude --resume {s}"),
                None => "claude --continue".into(),
            });
            pane.agent_session = session;
        }
        // Codex など: 再開の方法が分かるものだけ
        let other = p
            .agent
            .as_deref()
            .filter(|_| !p.claude)
            .and_then(|cmd| Some((cmd, agent_label(cmd)?)))
            .filter(|(_, label)| *label != CLAUDE);
        let mut other_restored = false;
        if let Some((cmd, _)) = other
            && self.settings.restore_claude
            && restore_claude_enabled()
        {
            let session = p
                .agent_session
                .clone()
                .filter(|s| crate::claude_session::valid_id(s));
            if let Some(line) = agent_resume_command(cmd, session.as_deref()) {
                pane.restore_cmds.push(line);
                pane.agent_session = session;
                other_restored = true;
            }
        }
        // 起動前でも状態を保持しておく（開かずに終了しても失われないように）
        pane.conda = p.conda;
        pane.agent = if p.claude {
            Some(CLAUDE)
        } else {
            other.filter(|_| other_restored).map(|(_, label)| label)
        };
        pane.activity_driven = pane.agent.is_some_and(|a| a != CLAUDE);
        pane.state = if pane.agent.is_some() {
            AgentState::Idle
        } else {
            AgentState::None
        };
        id
    }

    fn node_from_state(&mut self, state: NodeState) -> Node {
        match state {
            NodeState::Group { active, tabs } => {
                let mut ids: Vec<PaneId> = tabs.into_iter().map(|p| self.restore_pane(p)).collect();
                if ids.is_empty() {
                    ids.push(self.alloc_pane(None));
                }
                let gid = self.alloc_id();
                let active = active.min(ids.len() - 1);
                self.groups.insert(gid, Group { tabs: ids, active });
                Node::Leaf(gid)
            }
            NodeState::Pane(p) => {
                let id = self.restore_pane(p);
                let gid = self.alloc_id();
                self.groups.insert(gid, Group::new(id));
                Node::Leaf(gid)
            }
            NodeState::Split {
                axis,
                ratio,
                first,
                second,
            } => Node::Split {
                axis,
                ratio: ratio.clamp(0.1, 0.9),
                first: Box::new(self.node_from_state(*first)),
                second: Box::new(self.node_from_state(*second)),
            },
        }
    }

    // -----------------------------------------------------------------------
    // 表示
    // -----------------------------------------------------------------------

    fn workspace_name(&self, index: usize) -> String {
        let Some(ws) = self.workspaces.get(index) else {
            return String::new();
        };
        if let Some(name) = &ws.custom_name {
            return name.clone();
        }
        // フォーカス中のペインのフォルダ名。ブラウザなどフォルダがなければ、ほかのターミナルのもの
        self.ws_focused_pane(ws)
            .and_then(|id| self.panes.get(&id))
            .and_then(|p| p.cwd.as_deref())
            .or_else(|| {
                self.ws_panes(ws)
                    .iter()
                    .find_map(|id| self.panes.get(id).and_then(|p| p.cwd.as_deref()))
            })
            .and_then(folder_name)
            .unwrap_or_else(|| format!("ワークスペース {}", index + 1))
    }

    /// ワークスペースのエージェントの状態（いちばん目を引くもの）と、その表示名
    fn workspace_agent(&self, ws: &Workspace) -> (AgentState, &'static str) {
        let panes: Vec<&Pane> = self
            .ws_panes(ws)
            .iter()
            .filter_map(|id| self.panes.get(id))
            .collect();
        for s in [AgentState::Asking, AgentState::Working, AgentState::Idle] {
            if let Some(p) = panes.iter().find(|p| p.state == s) {
                return (s, p.agent.unwrap_or(CLAUDE));
            }
        }
        (AgentState::None, CLAUDE)
    }

    /// ワークスペースの丸の色（中のペインでいちばん目を引くべき状態）
    fn workspace_dot(&self, ws: &Workspace, selected: bool) -> Option<u32> {
        let best = self
            .ws_panes(ws)
            .iter()
            .filter_map(|id| self.panes.get(id))
            .map(|p| {
                // 表示中のワークスペースでは、見えているペインの「未確認」は数えない
                let attention = p.attention && !selected;
                (dot_priority(p.state, attention), p.state, attention)
            })
            .max_by_key(|(prio, _, _)| *prio)?;
        let (_, claude, attention) = best;
        let attention = attention || (ws.unread > 0 && claude == AgentState::None);
        pane_dot(claude, attention)
    }

    /// 状態を表す小さな丸（色は `pane_dot` を参照）
    fn status_dot(color: Option<u32>) -> Div {
        div()
            .flex_none()
            .size(px(7.))
            .rounded_full()
            .when_some(color, |el, c| el.bg(rgb(c)))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.workspaces.iter().enumerate().map(|(i, ws)| {
            let selected = i == self.active;
            let name = self.workspace_name(i);
            let focused = self.ws_focused_pane(ws).and_then(|id| self.panes.get(&id));
            let cwd = focused.and_then(|p| p.cwd.as_deref()).map(short_path);
            let branch = focused.and_then(|p| p.branch.clone());
            let conda = focused.and_then(|p| p.conda.clone());
            let (claude, agent) = self.workspace_agent(ws);
            let unread = ws.unread;
            let note = ws.last_notification.clone();
            let dot = self.workspace_dot(ws, selected);
            // 未読の数字も状態の色で（入力待ち=赤、完了=緑 …）
            let badge = dot.filter(|_| unread > 0).unwrap_or(th().accent);
            let (fg, meta) = if selected {
                (0xffffff, th().selected_meta)
            } else {
                (th().text, th().text_dim)
            };

            // 2 行目: ブランチ · フォルダ
            let mut line2 = Vec::new();
            if let Some(b) = branch {
                line2.push(format!("⎇ {b}"));
            }
            if let Some(c) = cwd {
                line2.push(c);
            }
            // 3 行目: conda · Claude の状態
            let mut line3: Vec<(String, u32)> = Vec::new();
            if let Some(env) = conda {
                line3.push((
                    format!("🐍 {env}"),
                    if selected { meta } else { th().green_dim },
                ));
            }
            match claude {
                AgentState::Asking => line3.push((
                    format!("{agent} 入力待ち"),
                    if selected { 0xffffff } else { th().red },
                )),
                AgentState::Working => line3.push((
                    format!("{agent} 実行中"),
                    if selected { 0xffffff } else { th().amber },
                )),
                AgentState::Idle => line3.push((format!("{agent} 待機中"), meta)),
                AgentState::None => {}
            }
            let n_tabs = self.ws_panes(ws).len();
            if n_tabs > 1 {
                line3.push((format!("{n_tabs} タブ"), meta));
            }

            div()
                .id(("ws", i))
                .group("ws-item")
                .flex()
                .flex_col()
                .gap(px(3.))
                .px(px(10.))
                // 上下の 2px は並べ替えの印（透明な枠）の分
                .py(px(6.))
                .rounded(px(8.))
                .cursor_pointer()
                .when(selected, |el| el.bg(rgb(th().selected_bg)))
                .when(!selected, |el| el.hover(|s| s.bg(rgb(th().hover_bg))))
                // タブをワークスペースに落とすと、そのワークスペースへ移動
                .drag_over::<DraggedTab>(|s, _, _, _| s.bg(rgb(th().hover_bg_strong)))
                .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                    this.move_tab(d.pane, TabDest::Workspace(i), window, cx);
                }))
                // ワークスペースをドラッグして並べ替え（落とした位置に入る）
                .on_drag(
                    DraggedWorkspace {
                        index: i,
                        label: name.clone(),
                    },
                    |d, _, _, cx| {
                        cx.new(|_| TabDragPreview {
                            label: d.label.clone(),
                        })
                    },
                )
                .border_color(gpui_kit::transparent_black())
                .border_t_2()
                .border_b_2()
                .drag_over::<DraggedWorkspace>(move |s, d, _, _| {
                    let accent = rgb(th().accent);
                    // 入る側の辺だけに線を出す（反対側の枠は消し、そのぶん余白を足して高さを保つ）
                    if d.index < i {
                        s.border_color(accent).border_t_0().pt(px(8.))
                    } else if d.index > i {
                        s.border_color(accent).border_b_0().pb(px(8.))
                    } else {
                        s
                    }
                })
                .on_drop(cx.listener(move |this, d: &DraggedWorkspace, window, cx| {
                    this.move_workspace(d.index, i, window, cx);
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        this.ws_menu = Some((i, ev.position));
                        cx.notify();
                    }),
                )
                .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                    if ev.click_count() >= 2 {
                        this.open_rename(i, window, cx);
                    } else {
                        this.select_workspace(i, window, cx);
                    }
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .child(Self::status_dot(dot))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(UI_TEXT))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(fg))
                                .child(name),
                        )
                        .when(unread > 0, |el| {
                            el.child(
                                div()
                                    .flex_none()
                                    .min_w(px(18.))
                                    .h(px(16.))
                                    .px(px(5.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .bg(rgb(badge))
                                    .text_color(rgb(th().chrome_bg))
                                    .child(unread.to_string()),
                            )
                        })
                        .child(
                            div()
                                .id(("ws-close", i))
                                .flex_none()
                                .invisible()
                                .group_hover("ws-item", |s| s.visible())
                                .px(px(4.))
                                .rounded(px(4.))
                                .text_size(px(11.))
                                .text_color(rgb(meta))
                                .hover(|s| s.bg(hsla(0., 0., 1., 0.12)).text_color(rgb(fg)))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.close_workspace(i, window, cx);
                                }))
                                .child("✕"),
                        ),
                )
                .when(!line2.is_empty(), |el| {
                    el.child(
                        div()
                            .pl(px(14.))
                            .truncate()
                            .text_size(px(UI_SMALL))
                            .text_color(rgb(meta))
                            .child(line2.join("  ·  ")),
                    )
                })
                .when(!line3.is_empty(), |el| {
                    el.child(
                        div()
                            .pl(px(14.))
                            .flex()
                            .gap(px(8.))
                            .text_size(px(UI_SMALL))
                            .children(
                                line3
                                    .into_iter()
                                    .map(|(t, c)| div().text_color(rgb(c)).child(t)),
                            ),
                    )
                })
                .when_some(note, |el, n| {
                    el.child(
                        div()
                            .pl(px(14.))
                            .truncate()
                            .text_size(px(UI_SMALL))
                            .text_color(rgb(if selected { meta } else { th().text_faint }))
                            .child(n),
                    )
                })
        });

        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .bg(theme::bg(th().chrome_bg))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(TAB_BAR_HEIGHT))
                    .flex_none()
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .flex()
                            .items_center()
                            .pl(px(16.))
                            .window_control_area(WindowControlArea::Drag)
                            .text_size(px(UI_SMALL))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(th().text_faint))
                            .child("WORKSPACES"),
                    )
                    .child(
                        div()
                            .id("ws-add")
                            .mr(px(10.))
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_size(px(15.))
                            .text_color(rgb(th().text_dim))
                            .hover(|s| s.bg(rgb(th().hover_bg)).text_color(rgb(th().text)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.new_workspace(&NewWorkspace, window, cx);
                            }))
                            .child("+"),
                    ),
            )
            .child(
                div()
                    .id("ws-list")
                    // 一覧の空いているところにタブを落とすと、新しいワークスペースになる
                    .on_drop(cx.listener(|this, d: &DraggedTab, window, cx| {
                        this.move_tab(d.pane, TabDest::NewWorkspace, window, cx);
                    }))
                    // ワークスペースを一覧の空いているところに落とすと、いちばん下へ
                    .on_drop(cx.listener(|this, d: &DraggedWorkspace, window, cx| {
                        let last = this.workspaces.len().saturating_sub(1);
                        this.move_workspace(d.index, last, window, cx);
                    }))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .px(px(8.))
                    .children(items),
            )
            .children(self.render_update_notice(cx))
            .children(self.render_usage())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .pl(px(16.))
                    .pr(px(10.))
                    .py(px(8.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(th().text_faint))
                            .child("Ctrl+Shift+G でフォルダ移動"),
                    )
                    .child(
                        div()
                            .id("open-settings")
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .px(px(8.))
                            .py(px(4.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_size(px(UI_SMALL))
                            .text_color(rgb(th().text_dim))
                            .hover(|s| s.bg(rgb(th().hover_bg)).text_color(rgb(th().text)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_settings(&OpenSettings, window, cx);
                            }))
                            .child("⚙")
                            .child("設定"),
                    ),
            )
    }

    // -----------------------------------------------------------------------
    // アップデート
    // -----------------------------------------------------------------------

    /// ［更新］: インストーラーをダウンロードして上書きインストールし、このアプリは終了する
    fn start_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let UpdateState::Available(release) = self.update.clone() else {
            return;
        };
        if !crate::update::CAN_INSTALL || release.installer.is_none() {
            crate::open::open_url(&release.page);
            return;
        }
        self.update = UpdateState::Installing(release.clone());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let r = release.clone();
            let result = cx
                .background_executor()
                .spawn(async move { crate::update::download_and_install(&r) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(()) => {
                    // インストーラーが上書きできるよう、保存してから終了する
                    this.closing = true;
                    this.save_now();
                    crate::window_state::save(&crate::window_state::SavedWindow::from_bounds(
                        window.window_bounds(),
                    ));
                    cx.quit();
                }
                Err(e) => {
                    log::warn!("アップデートできませんでした: {e}");
                    this.update = UpdateState::Failed(release, e);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// サイドバーの下の、Claude・Codex の利用上限（5 時間枠・週）の棒
    fn render_usage(&self) -> Option<AnyElement> {
        let now = crate::hooks::now_secs();
        let rows: Vec<(String, crate::ipc::LimitWindow)> = self
            .usage
            .iter()
            .flat_map(|(agent, u)| {
                [
                    (format!("{agent} 5時間"), u.five_hour),
                    (format!("{agent} 週"), u.seven_day),
                ]
            })
            .filter_map(|(label, w)| Some((label, w.filter(|w| w.resets_at > now)?)))
            .collect();
        if rows.is_empty() {
            return None;
        }
        let c = th();
        let row = |label: String, w: crate::ipc::LimitWindow| {
            let pct = w.used_percentage.clamp(0., 100.);
            let color = if pct >= 90. {
                c.red
            } else if pct >= 70. {
                c.amber
            } else {
                c.accent
            };
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .text_size(px(11.))
                        .child(
                            div()
                                .text_color(rgb(c.text_dim))
                                .child(format!("{label}  {pct:.0}%")),
                        )
                        .child(
                            div()
                                .text_color(rgb(c.text_faint))
                                .child(crate::hooks::until_reset(w.resets_at, now)),
                        ),
                )
                .child(
                    div()
                        .h(px(4.))
                        .w_full()
                        .rounded(px(2.))
                        .bg(rgb(c.hover_bg_strong))
                        .child(
                            div()
                                .h_full()
                                .rounded(px(2.))
                                .w(relative((pct / 100.) as f32))
                                .bg(rgb(color)),
                        ),
                )
        };
        Some(
            div()
                .mx(px(12.))
                .mb(px(4.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .children(rows.into_iter().map(|(l, w)| row(l, w)))
                .into_any_element(),
        )
    }

    /// サイドバーの下のお知らせ「新しいバージョン vX があります ［更新］」
    fn render_update_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = th();
        let (release, text, button): (_, String, Option<&'static str>) = match &self.update {
            UpdateState::Available(r) => (
                r,
                format!("新しいバージョン v{}", r.version),
                Some(if crate::update::CAN_INSTALL && r.installer.is_some() {
                    "更新"
                } else {
                    "開く"
                }),
            ),
            UpdateState::Installing(r) => {
                (r, format!("v{} をダウンロードしています…", r.version), None)
            }
            UpdateState::Failed(r, _) => {
                (r, "更新できませんでした".to_string(), Some("ページを開く"))
            }
            UpdateState::None | UpdateState::Dismissed => return None,
        };
        let page = release.page.clone();
        let failed = matches!(self.update, UpdateState::Failed(..));
        let installing = matches!(self.update, UpdateState::Installing(_));
        Some(
            div()
                .mx(px(8.))
                .mb(px(4.))
                .px(px(10.))
                .py(px(8.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .rounded(px(8.))
                .bg(rgb(c.hover_bg))
                .border_1()
                .border_color(rgb(c.accent))
                .child(
                    div()
                        .text_size(px(UI_SMALL))
                        .text_color(rgb(c.text))
                        .child(text),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .when_some(button, |el, label| {
                            el.child(
                                div()
                                    .id("update-run")
                                    .px(px(10.))
                                    .py(px(3.))
                                    .rounded(px(5.))
                                    .cursor_pointer()
                                    .bg(rgb(c.accent))
                                    .text_size(px(UI_SMALL))
                                    .text_color(rgb(0xffffff))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if failed {
                                            if let UpdateState::Failed(r, _) = &this.update {
                                                crate::open::open_url(&r.page);
                                            }
                                        } else {
                                            this.start_update(window, cx);
                                        }
                                    }))
                                    .child(label),
                            )
                        })
                        .when(!page.is_empty() && !failed, |el| {
                            el.child(
                                div()
                                    .id("update-notes")
                                    .px(px(6.))
                                    .py(px(3.))
                                    .rounded(px(5.))
                                    .cursor_pointer()
                                    .text_size(px(UI_SMALL))
                                    .text_color(rgb(c.text_dim))
                                    .hover(|s| s.bg(rgb(c.hover_bg_strong)).text_color(rgb(c.text)))
                                    .on_click(move |_, _, _| crate::open::open_url(&page))
                                    .child("変更点"),
                            )
                        })
                        .child(div().flex_1())
                        .when(!installing, |el| {
                            el.child(
                                div()
                                    .id("update-dismiss")
                                    .flex_none()
                                    .px(px(4.))
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .text_size(px(11.))
                                    .text_color(rgb(c.text_faint))
                                    .hover(|s| s.bg(rgb(c.hover_bg_strong)).text_color(rgb(c.text)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.update = UpdateState::Dismissed;
                                        cx.notify();
                                    }))
                                    .child("✕"),
                            )
                        }),
                )
                .into_any_element(),
        )
    }

    // -----------------------------------------------------------------------
    // 設定画面
    // -----------------------------------------------------------------------

    fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = th();
        let section = |title: &'static str| {
            div()
                .pt(px(18.))
                .pb(px(8.))
                .text_size(px(UI_SMALL))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(c.text_dim))
                .child(title)
        };

        // テーマ: 各テーマの配色でミニプレビューを描く
        let current_theme = self.settings.theme.clone();
        let theme_cards = THEMES.iter().enumerate().map(|(i, def)| {
            let tc = derive(def);
            let selected = def.name.eq_ignore_ascii_case(&current_theme);
            let p = &def.palette;
            let dots = (1..7).map(|k| div().size(px(8.)).rounded_full().bg(rgb(hex(p.ansi[k]))));
            let name = def.name;
            div()
                .id(("theme", i))
                .w(px(176.))
                .flex()
                .flex_col()
                .overflow_hidden()
                .rounded(px(8.))
                .border_2()
                .border_color(if selected {
                    rgb(c.accent)
                } else {
                    rgb(c.panel_border)
                })
                .cursor_pointer()
                .hover(|s| s.border_color(rgb(c.accent)))
                .on_click(cx.listener(move |this, _, window, cx| this.set_theme(name, window, cx)))
                .child(
                    div()
                        .flex()
                        .h(px(64.))
                        .child(
                            // サイドバー部分
                            div()
                                .w(px(36.))
                                .h_full()
                                .bg(rgb(tc.chrome_bg))
                                .p(px(5.))
                                .child(div().h(px(9.)).rounded(px(3.)).bg(rgb(tc.selected_bg))),
                        )
                        .child(
                            // ターミナル部分
                            div()
                                .flex_1()
                                .h_full()
                                .bg(rgb(tc.term_bg))
                                .px(px(8.))
                                .py(px(7.))
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .flex()
                                        .gap(px(4.))
                                        .text_size(px(10.))
                                        .child(
                                            div().text_color(rgb(hex(p.ansi[2]))).child("~/wimux"),
                                        )
                                        .child(div().text_color(rgb(tc.text)).child("$ claude")),
                                )
                                .child(div().flex().gap(px(4.)).children(dots)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px(px(10.))
                        .py(px(6.))
                        .bg(rgb(c.menu_bg))
                        .text_size(px(UI_SMALL))
                        .text_color(rgb(c.text))
                        .child(name)
                        .when(selected, |el| {
                            el.child(div().text_color(rgb(c.accent)).child("✓"))
                        }),
                )
        });

        // フォント
        let current_font = self.settings.font_family.clone();
        let mut fonts: Vec<Option<&'static str>> = vec![None];
        fonts.extend(installed_fonts(cx).into_iter().map(Some));
        let font_chips = fonts.into_iter().enumerate().map(|(i, f)| {
            let selected = current_font.as_deref() == f;
            let label = f.unwrap_or("自動");
            div()
                .id(("font", i))
                .px(px(10.))
                .py(px(5.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(UI_SMALL))
                .when(f.is_some(), |el| el.font_family(label))
                .when(selected, |el| {
                    el.bg(rgb(c.selected_bg)).text_color(rgb(0xffffff))
                })
                .when(!selected, |el| {
                    el.bg(rgb(c.hover_bg))
                        .text_color(rgb(c.text))
                        .hover(|s| s.bg(rgb(c.hover_bg_strong)))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_font_family(f.map(str::to_string), cx)
                }))
                .child(label)
        });

        // 選択肢のチップ（透明度・行の高さ・カーソルなど）
        let chip = |id: ElementId, label: String, selected: bool| {
            div()
                .id(id)
                .px(px(10.))
                .py(px(5.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(UI_SMALL))
                .when(selected, |el| {
                    el.bg(rgb(c.selected_bg)).text_color(rgb(0xffffff))
                })
                .when(!selected, |el| {
                    el.bg(rgb(c.hover_bg))
                        .text_color(rgb(c.text))
                        .hover(|s| s.bg(rgb(c.hover_bg_strong)))
                })
                .child(label)
        };
        let hint = |text: &'static str| {
            div()
                .pt(px(6.))
                .text_size(px(11.))
                .text_color(rgb(c.text_faint))
                .child(text)
        };

        // 背景の透過
        let opacity = self.settings.opacity;
        let opacity_chips = [1.0f32, 0.95, 0.9, 0.85, 0.8, 0.75, 0.7, 0.6, 0.5]
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                let label = if v >= 0.999 {
                    "なし".to_string()
                } else {
                    format!("{}%", (v * 100.).round() as i32)
                };
                chip(("opacity", i).into(), label, (opacity - v).abs() < 0.001).on_click(
                    cx.listener(move |this, _, window, cx| this.set_opacity(v, window, cx)),
                )
            });
        let backdrop = self.settings.backdrop;
        let backdrop_chips = [
            (Backdrop::Transparent, "そのまま透かす"),
            (Backdrop::Blur, "すりガラス（ぼかす）"),
            (Backdrop::Mica, "Mica"),
            (Backdrop::MicaAlt, "Mica Alt"),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (b, label))| {
            chip(("backdrop", i).into(), label.to_string(), backdrop == b)
                .on_click(cx.listener(move |this, _, window, cx| this.set_backdrop(b, window, cx)))
        });

        // 行の高さ
        let line_height = self.settings.line_height;
        let line_height_chips = [1.0f32, 1.1, 1.2, 1.25, 1.3, 1.4, 1.5, 1.6]
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                chip(
                    ("line-height", i).into(),
                    format!("{v}"),
                    (line_height - v).abs() < 0.001,
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.set_line_height(v, window, cx)),
                )
            });

        // カーソルの形
        let cursor = self.settings.cursor;
        let cursor_chips = [
            (CursorKind::Block, "█ ブロック"),
            (CursorKind::Beam, "▏ 縦線"),
            (CursorKind::Underline, "▁ 下線"),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (k, label))| {
            chip(("cursor", i).into(), label.to_string(), cursor == k)
                .on_click(cx.listener(move |this, _, window, cx| this.set_cursor(k, window, cx)))
        });

        let step_button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .bg(rgb(c.hover_bg))
                .text_color(rgb(c.text))
                .hover(|s| s.bg(rgb(c.hover_bg_strong)))
                .child(label)
        };

        let restore = self.settings.restore_claude;
        let toggle = div()
            .id("toggle-restore-claude")
            .flex_none()
            .w(px(38.))
            .h(px(22.))
            .p(px(3.))
            .flex()
            .rounded_full()
            .cursor_pointer()
            .bg(if restore {
                rgb(c.accent)
            } else {
                rgb(c.hover_bg_strong)
            })
            .when(restore, |el| el.justify_end())
            .on_click(cx.listener(|this, _, _, cx| {
                this.settings.restore_claude = !this.settings.restore_claude;
                settings::save(&this.settings);
                cx.notify();
            }))
            .child(div().size(px(16.)).rounded_full().bg(rgb(0xffffff)));

        let cdp = self.settings.browser_cdp;
        let cdp_toggle = div()
            .id("toggle-browser-cdp")
            .w(px(38.))
            .h(px(22.))
            .p(px(3.))
            .flex()
            .flex_none()
            .rounded_full()
            .cursor_pointer()
            .bg(if cdp {
                rgb(c.accent)
            } else {
                rgb(c.hover_bg_strong)
            })
            .when(cdp, |el| el.justify_end())
            .on_click(cx.listener(|this, _, _, cx| {
                this.settings.browser_cdp = !this.settings.browser_cdp;
                settings::save(&this.settings);
                cx.notify();
            }))
            .child(div().size(px(16.)).rounded_full().bg(rgb(0xffffff)));

        let check_updates = self.settings.check_updates;
        let update_toggle = div()
            .id("toggle-check-updates")
            .w(px(38.))
            .h(px(22.))
            .p(px(3.))
            .flex()
            .flex_none()
            .rounded_full()
            .cursor_pointer()
            .bg(if check_updates {
                rgb(c.accent)
            } else {
                rgb(c.hover_bg_strong)
            })
            .when(check_updates, |el| el.justify_end())
            .on_click(cx.listener(|this, _, _, cx| {
                this.settings.check_updates = !this.settings.check_updates;
                settings::save(&this.settings);
                cx.notify();
            }))
            .child(div().size(px(16.)).rounded_full().bg(rgb(0xffffff)));

        let row = |label: &'static str, hint: &'static str| {
            // 説明が長くても右のトグルを押し出さないよう、縮めて折り返す
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(UI_TEXT))
                        .text_color(rgb(c.text))
                        .child(label),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(c.text_faint))
                        .child(hint),
                )
        };

        let panel = div()
            .id("settings-panel")
            .track_focus(&self.settings_focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" {
                    this.close_settings(window, cx);
                }
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(600.))
            .max_w(relative(0.92))
            .max_h(relative(0.86))
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(rgb(c.chrome_bg))
            .border_1()
            .border_color(rgb(c.panel_border))
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .px(px(20.))
                    .h(px(48.))
                    .border_b_1()
                    .border_color(rgb(c.panel_border))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c.text))
                            .child("設定"),
                    )
                    .child(
                        div()
                            .id("settings-close")
                            .size(px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_color(rgb(c.text_dim))
                            .hover(|s| s.bg(rgb(c.hover_bg)).text_color(rgb(c.text)))
                            .on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx)))
                            .child("✕"),
                    ),
            )
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(20.))
                    .pb(px(20.))
                    .child(section("テーマ"))
                    .child(div().flex().flex_wrap().gap(px(10.)).children(theme_cards))
                    .child(section("背景の透過"))
                    .child(div().flex().flex_wrap().gap(px(6.)).children(opacity_chips))
                    .when(opacity < 0.999, |el| {
                        el.child(
                            div()
                                .pt(px(10.))
                                .flex()
                                .flex_wrap()
                                .gap(px(6.))
                                .children(backdrop_chips),
                        )
                    })
                    .child(hint(
                        "ターミナル・サイドバー・タブの行の背景を透かします。後ろのウィンドウが見えるのは「そのまま透かす」だけです（すりガラスは後ろをぼかし、Mica / Mica Alt は壁紙の色だけが出ます。Mica は Windows 11 のみ）",
                    ))
                    .child(section("フォント"))
                    .child(div().flex().flex_wrap().gap(px(6.)).children(font_chips))
                    .child(
                        div()
                            .pt(px(6.))
                            .text_size(px(11.))
                            .text_color(rgb(c.text_faint))
                            .child("インストールされている等幅フォントだけを表示しています。自動は JetBrains Mono → Cascadia Mono → … の順"),
                    )
                    .child(section("文字サイズ"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(step_button("font-smaller", "−").on_click(cx.listener(
                                |this, _, _, cx| this.set_font_size(this.font_size - 1., cx),
                            )))
                            .child(
                                div()
                                    .w(px(36.))
                                    .text_center()
                                    .text_size(px(UI_TEXT))
                                    .text_color(rgb(c.text))
                                    .child(format!("{}", self.font_size)),
                            )
                            .child(step_button("font-larger", "+").on_click(cx.listener(
                                |this, _, _, cx| this.set_font_size(this.font_size + 1., cx),
                            )))
                            .child(step_button("font-reset", "↺").on_click(cx.listener(
                                |this, _, _, cx| this.set_font_size(DEFAULT_FONT_SIZE, cx),
                            )))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(c.text_faint))
                                    .child("Ctrl+= / Ctrl+- / Ctrl+0 でも変更できます"),
                            ),
                    )
                    .child(section("行の高さ"))
                    .child(div().flex().flex_wrap().gap(px(6.)).children(line_height_chips))
                    .child(section("カーソル"))
                    .child(div().flex().flex_wrap().gap(px(6.)).children(cursor_chips))
                    .child(hint("vim の挿入モードなど、アプリが形を指定したときはそちらが優先されます"))
                    .child(section("ブラウザ"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(16.))
                            .child(row(
                                "AI からブラウザを操作できるようにする",
                                "Sondev のペインで起動した Claude Code は、自動で Sondev のブラウザを操作できます（MCP「sondev-browser」。Node.js が必要。再起動後に反映）",
                            ))
                            .child(cdp_toggle),
                    )
                    .child(section("Claude Code"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(16.))
                            .child(row(
                                "起動時に会話を再開する",
                                "前回 Claude Code・Codex などを開いていたペインで、その会話を再開します（claude --resume / codex resume）",
                            ))
                            .child(toggle),
                    )
                    .child(section("アップデート"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(16.))
                            .child(row(
                                "新しいバージョンを確認する",
                                "起動時に GitHub のリリースを確かめ、新しい版があればサイドバーに［更新］を出します",
                            ))
                            .child(update_toggle),
                    )
                    .child(hint(concat!(
                        "今のバージョン: v",
                        env!("CARGO_PKG_VERSION")
                    ))),
            );

        div()
            .id("settings-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .items_center()
            .bg(hsla(0., 0., 0., 0.5))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_settings(window, cx)),
            )
            .child(panel)
    }

    /// ワークスペースの右クリックメニュー
    fn render_ws_menu(
        &self,
        index: usize,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let item = |id: &'static str, label: &'static str, color: u32| {
            div()
                .id(id)
                .flex()
                .items_center()
                .h(px(28.))
                .px(px(10.))
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(px(UI_TEXT))
                .text_color(rgb(color))
                .hover(|s| s.bg(rgb(th().hover_bg_strong)))
                .child(label)
        };
        let close_menu = |this: &mut Self, cx: &mut Context<Self>| {
            this.ws_menu = None;
            cx.notify();
        };

        div()
            .id("ws-menu-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            // メニューの外をクリックしたら閉じる
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| close_menu(this, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| close_menu(this, cx)),
            )
            .child(
                div()
                    .id("ws-menu")
                    .absolute()
                    .left(pos.x)
                    .top(pos.y)
                    .w(px(200.))
                    .p(px(4.))
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .rounded(px(8.))
                    .bg(rgb(th().menu_bg))
                    .border_1()
                    .border_color(rgb(th().panel_border))
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .px(px(10.))
                            .pt(px(4.))
                            .pb(px(4.))
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(th().text_faint))
                            .child(self.workspace_name(index)),
                    )
                    .child(
                        item("ws-menu-rename", "名前を変更", th().text).on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.ws_menu = None;
                                this.open_rename(index, window, cx);
                            },
                        )),
                    )
                    .child(
                        item("ws-menu-new", "新しいワークスペース", th().text).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.ws_menu = None;
                                this.new_workspace(&NewWorkspace, window, cx);
                            }),
                        ),
                    )
                    .when(index > 0, |el| {
                        el.child(
                            item("ws-menu-up", "上へ移動", th().text).on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.ws_menu = None;
                                    this.move_workspace(index, index - 1, window, cx);
                                },
                            )),
                        )
                    })
                    .when(index + 1 < self.workspaces.len(), |el| {
                        el.child(
                            item("ws-menu-down", "下へ移動", th().text).on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.ws_menu = None;
                                    this.move_workspace(index, index + 1, window, cx);
                                },
                            )),
                        )
                    })
                    .child(div().my(px(3.)).h(px(1.)).bg(rgb(th().panel_border)))
                    .child(item("ws-menu-delete", "削除", DANGER).on_click(cx.listener(
                        move |this, _, window, cx| {
                            this.ws_menu = None;
                            this.close_workspace(index, window, cx);
                        },
                    ))),
            )
    }

    /// 区画ごとのタブの行
    /// `edge`: この区画がウィンドウの上端 / 右上の角に接しているか。
    /// 上端の区画はタブ行の空きでウィンドウをドラッグでき、右上の区画にはウィンドウボタンを置く
    fn render_group_tabs(
        &self,
        gid: GroupId,
        focused_group: bool,
        edge: Edge,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let group = self.groups.get(&gid);
        let tabs: Vec<PaneId> = group.map(|g| g.tabs.clone()).unwrap_or_default();
        let active_index = group.map_or(0, |g| g.active);

        let items = tabs.iter().enumerate().map(|(i, id)| {
            let active = i == active_index;
            let pane = self.panes.get(id);
            let label = match pane {
                Some(p) if p.is_browser() => {
                    browser::short_label(p.title.as_deref(), p.url.as_deref().unwrap_or(""))
                }
                _ => pane
                    .and_then(|p| p.cwd.as_deref().and_then(folder_name))
                    .unwrap_or_else(|| format!("タブ {}", i + 1)),
            };
            let claude = pane.map_or(AgentState::None, |p| p.state);
            let attention = pane.is_some_and(|p| p.attention);
            let dot = pane_dot(claude, attention);
            let id = *id;
            let dragged = DraggedTab {
                pane: id,
                label: label.clone(),
            };
            let accent = th().accent;

            div()
                .id(("tab", id))
                // ドラッグで並べ替え・別の区画へ移動・分割して配置
                .on_drag(dragged, |d, _, _, cx| {
                    cx.new(|_| TabDragPreview {
                        label: d.label.clone(),
                    })
                })
                .drag_over::<DraggedTab>(move |s, _, _, _| s.border_l_2().border_color(rgb(accent)))
                .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                    this.move_tab(d.pane, TabDest::Tab { gid, index: i }, window, cx);
                }))
                .group("tab")
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(7.))
                .h(px(TAB_HEIGHT))
                .pl(px(12.))
                .pr(px(6.))
                .rounded_t(px(7.))
                .cursor_pointer()
                .text_size(px(UI_TAB))
                .when(active, |el| {
                    el.bg(theme::bg(th().term_bg))
                        .text_color(rgb(if focused_group {
                            th().text
                        } else {
                            th().text_dim
                        }))
                })
                .when(!active, |el| {
                    el.text_color(rgb(th().text_faint))
                        .hover(|s| s.bg(rgb(th().hover_bg)).text_color(rgb(th().text_dim)))
                })
                // フォーカスしている区画の表示中タブには上に青い線
                .when(active && focused_group, |el| {
                    el.child(
                        div()
                            .absolute()
                            .top_0()
                            .left(px(8.))
                            .right(px(8.))
                            .h(px(2.))
                            .rounded_b(px(2.))
                            .bg(rgb(th().accent)),
                    )
                })
                .on_click(
                    cx.listener(move |this, _, window, cx| this.select_tab(gid, i, window, cx)),
                )
                .when_some(dot, |el, c| el.child(Self::status_dot(Some(c))))
                .child(div().max_w(px(160.)).truncate().child(label))
                .child(
                    div()
                        .id(("tab-close", id))
                        .size(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .text_size(px(10.))
                        .text_color(rgb(th().text_dim))
                        .when(!active, |el| {
                            el.invisible().group_hover("tab", |s| s.visible())
                        })
                        .hover(|s| s.bg(rgb(th().hover_bg_strong)).text_color(rgb(th().text)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_pane_id(id, window, cx);
                        }))
                        .child("✕"),
                )
        });

        let small_button = |id: (&'static str, usize), label: &'static str| {
            div()
                .id(id)
                .mb(px(3.))
                .size(px(24.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .text_color(rgb(th().text_faint))
                .hover(|s| s.bg(rgb(th().hover_bg)).text_color(rgb(th().text)))
                .child(label)
        };

        let dragging = self.tab_dragging(cx);
        div()
            .id(("group-tabs", gid))
            // タブの行の空いているところに落とすと最後に加える
            .on_drop(cx.listener(move |this, d: &DraggedTab, window, cx| {
                this.move_tab(
                    d.pane,
                    TabDest::Tab {
                        gid,
                        index: usize::MAX,
                    },
                    window,
                    cx,
                );
            }))
            .drag_over::<DraggedTab>(|s, _, _, _| s.bg(rgb(th().hover_bg)))
            .flex()
            .flex_none()
            .items_end()
            .gap(px(2.))
            .h(px(TAB_BAR_HEIGHT))
            .pl(px(4.))
            .when(!(edge.top_right && custom_titlebar()), |el| el.pr(px(4.)))
            .bg(theme::bg(th().chrome_bg))
            .child(
                div()
                    .id(("tabs-scroll", gid))
                    .flex()
                    .items_end()
                    .gap(px(2.))
                    .min_w_0()
                    .overflow_x_scroll()
                    .children(items),
            )
            .child(
                small_button(("tab-add", gid), "+")
                    .text_size(px(15.))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.new_tab_in(gid, window, cx)),
                    ),
            )
            // 空いているところ（上端の区画ならドラッグでウィンドウ移動、ダブルクリックで最大化）
            // （タブのドラッグ中はウィンドウ移動の領域にしない。落とせるように）
            .child(div().flex_1().h_full().when(edge.top && !dragging, |el| {
                el.window_control_area(WindowControlArea::Drag)
            }))
            // 区画を左右 / 上下に分割
            .child(
                small_button(("split-right", gid), "◫")
                    .text_size(px(13.))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.focus_group(gid);
                        this.split(Axis::Horizontal, window, cx);
                    })),
            )
            .child(
                small_button(("split-down", gid), "⊟")
                    .text_size(px(13.))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.focus_group(gid);
                        this.split(Axis::Vertical, window, cx);
                    })),
            )
            .when(edge.top_right && custom_titlebar(), |el| {
                el.child(Self::caption_buttons(window))
            })
    }

    fn focus_group(&mut self, gid: GroupId) {
        if let Some(ws) = self.workspaces.get_mut(self.active) {
            ws.focused = gid;
        }
    }

    /// Windows 風の最小化・最大化・閉じるボタン（独自タイトルバー用）
    fn caption_buttons(window: &Window) -> impl IntoElement {
        let button =
            |id: &'static str, glyph: &'static str, area: WindowControlArea, close: bool| {
                div()
                    .id(id)
                    .w(px(46.))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_family(CAPTION_FONT)
                    .text_size(px(10.))
                    .text_color(rgb(th().text_dim))
                    .window_control_area(area)
                    .when(close, |el| {
                        el.hover(|s| s.bg(rgb(CLOSE_RED)).text_color(rgb(0xffffff)))
                    })
                    .when(!close, |el| {
                        el.hover(|s| s.bg(rgb(th().hover_bg)).text_color(rgb(th().text)))
                    })
                    .child(glyph)
            };
        let max_glyph = if window.is_maximized() {
            "\u{E923}"
        } else {
            "\u{E922}"
        };
        div()
            .flex()
            .flex_none()
            .h_full()
            .child(button("min", "\u{E921}", WindowControlArea::Min, false))
            .child(button("max", max_glyph, WindowControlArea::Max, false))
            .child(button("close", "\u{E8BB}", WindowControlArea::Close, true))
    }

    /// ブラウザのタブ: 操作バー＋ WebView を重ねる領域
    fn render_browser(&self, id: PaneId, cx: &App) -> AnyElement {
        let c = th();
        let pane = &self.panes[&id];
        let url_field = pane.url_field.clone();
        let blank = pane.url.as_deref().is_none_or(str::is_empty);
        let loading = pane.loading;
        let browser = pane.browser.clone();
        let error = pane.browser_error.clone();
        // sync_browsers と同じ条件にする（違うと 1 フレームごとに隠す・表示を繰り返してチカチカする）
        let show = !self.overlay_open() && !self.tab_dragging(cx);

        let nav_button =
            |key: &'static str, label: &'static str, b: Option<Rc<Browser>>, f: fn(&Browser)| {
                div()
                    .id((key, id))
                    .size(px(26.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_color(rgb(c.text_dim))
                    .hover(|s| s.bg(rgb(c.hover_bg)).text_color(rgb(c.text)))
                    .on_click(move |_, _, _| {
                        if let Some(b) = &b {
                            f(b);
                        }
                    })
                    .child(label)
            };

        let toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .h(px(36.))
            .px(px(6.))
            .border_b_1()
            .border_color(rgb(c.divider))
            .child(nav_button("web-back", "←", browser.clone(), Browser::back))
            .child(nav_button(
                "web-forward",
                "→",
                browser.clone(),
                Browser::forward,
            ))
            .child(nav_button(
                "web-reload",
                if loading { "✕" } else { "↻" },
                browser.clone(),
                Browser::reload,
            ))
            .child(
                div()
                    .id(("web-url", id))
                    .flex_1()
                    .min_w_0()
                    .h(px(26.))
                    .ml(px(4.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .bg(rgb(c.chrome_bg))
                    .text_size(px(UI_SMALL))
                    .when_some(url_field, |el, f| el.child(f)),
            );

        // WebView を重ねる領域。描画のたびに位置を合わせる
        let area = div()
            .flex_1()
            .min_h_0()
            .relative()
            .child(
                div()
                    .absolute()
                    .top(px(16.))
                    .left(px(16.))
                    .text_size(px(UI_SMALL))
                    .text_color(rgb(c.text_faint))
                    .child(match (&browser, &error) {
                        (Some(_), _) => String::new(),
                        (None, Some(e)) => format!("ブラウザを開けませんでした: {e}"),
                        (None, None) if blank => {
                            "上の欄に URL を入力して Enter（例: 3000 → localhost:3000）".into()
                        }
                        (None, None) => "ブラウザを準備しています…".into(),
                    }),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        if let Some(b) = &browser {
                            b.set_visible(show);
                            b.set_bounds(bounds, window.scale_factor());
                        }
                    },
                )
                .absolute()
                .size_full(),
            );

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(toolbar)
            .child(area)
            .into_any_element()
    }

    fn render_node(
        &self,
        node: &Node,
        path: Vec<bool>,
        focused_group: GroupId,
        rects: &HashMap<GroupId, crate::workspace::Rect>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            Node::Leaf(gid) => {
                let is_focused = *gid == focused_group;
                let pane_id = self.groups.get(gid).and_then(|g| g.active_pane());
                let pane = pane_id.and_then(|id| self.panes.get(&id));
                let view = pane.and_then(|p| p.view.clone());
                // 見ていない間に何かあった区画は、状態の色の枠で囲む
                let attention = pane
                    .filter(|p| p.attention)
                    .and_then(|p| pane_dot(p.state, true));
                let browser_el = match (pane_id, pane) {
                    (Some(id), Some(p)) if p.is_browser() => Some(self.render_browser(id, cx)),
                    _ => None,
                };
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(self.render_group_tabs(
                        *gid,
                        is_focused,
                        Edge::of(rects.get(gid)),
                        window,
                        cx,
                    ))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .bg(theme::bg(th().term_bg))
                            .border_2()
                            .border_color(match attention {
                                Some(a) => rgb(a).into(),
                                None => transparent_black(),
                            })
                            .when_some(view, |el, v| el.child(v))
                            .when_some(browser_el, |el, b| el.child(b))
                            .when(self.tab_dragging(cx), |el| {
                                el.child(self.render_drop_overlay(*gid, cx))
                            }),
                    )
                    .into_any_element()
            }
            Node::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let mut first_path = path.clone();
                first_path.push(false);
                let mut second_path = path.clone();
                second_path.push(true);
                let first_el =
                    self.render_node(first, first_path, focused_group, rects, window, cx);
                let second_el =
                    self.render_node(second, second_path, focused_group, rects, window, cx);
                let horizontal = *axis == Axis::Horizontal;
                let axis = *axis;
                // 境界をつかむところ（線は 1px だが、つかめる幅は広げる）。区画より上に重ねる
                const GRIP: f32 = 7.;
                let handle_id: SharedString = format!(
                    "divider-{}",
                    path.iter()
                        .map(|b| if *b { '1' } else { '0' })
                        .collect::<String>()
                )
                .into();
                let handle = div()
                    .id(handle_id)
                    .absolute()
                    .occlude()
                    .when(horizontal, |el| {
                        el.top_0()
                            .bottom_0()
                            .left(relative(*ratio))
                            .ml(px(-(GRIP - 1.) / 2.))
                            .w(px(GRIP))
                            .cursor_col_resize()
                    })
                    .when(!horizontal, |el| {
                        el.left_0()
                            .right_0()
                            .top(relative(*ratio))
                            .mt(px(-(GRIP - 1.) / 2.))
                            .h(px(GRIP))
                            .cursor_row_resize()
                    })
                    .hover(|s| s.bg(rgba((th().accent << 8) | 0x60)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.resizing = true;
                            cx.stop_propagation();
                        }),
                    )
                    .on_drag(DraggedDivider { path: path.clone() }, |_, _, _, cx| {
                        cx.new(|_| DividerPreview)
                    });
                let first_box = div()
                    .flex_none()
                    .min_w_0()
                    .min_h_0()
                    .when(horizontal, |el| el.h_full().w(relative(*ratio)))
                    .when(!horizontal, |el| el.w_full().h(relative(*ratio)))
                    .child(first_el);
                let divider = div()
                    .flex_none()
                    .bg(rgb(th().divider))
                    .when(horizontal, |el| el.w(px(1.)).h_full())
                    .when(!horizontal, |el| el.h(px(1.)).w_full());
                let second_box = div().flex_1().min_w_0().min_h_0().child(second_el);
                div()
                    .id(SharedString::from(format!("split-{path:?}")))
                    .relative()
                    .flex()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .when(horizontal, |el| el.flex_row())
                    .when(!horizontal, |el| el.flex_col())
                    .on_drag_move(cx.listener(
                        move |this, ev: &DragMoveEvent<DraggedDivider>, _, cx| {
                            this.on_divider_drag(&path, axis, ev, cx);
                        },
                    ))
                    .child(first_box)
                    .child(divider)
                    .child(second_box)
                    .child(handle)
                    .into_any_element()
            }
        }
    }
}

impl Render for WimuxWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 表示する（各区画で選ばれている）タブだけ起動する（遅延起動）
        let (root, focused_group) = match self.workspaces.get(self.active) {
            Some(ws) => (ws.root.clone(), ws.focused),
            None => (Node::Leaf(0), 0),
        };
        let visible: Vec<PaneId> = root
            .leaves()
            .iter()
            .filter_map(|g| self.groups.get(g).and_then(|g| g.active_pane()))
            .collect();
        for id in &visible {
            self.ensure_view(*id, window, cx);
        }
        self.sync_browsers(&visible, window, cx);
        window.set_window_title(&format!(
            "{} — {}",
            crate::APP_NAME,
            self.workspace_name(self.active)
        ));

        let rects: HashMap<GroupId, crate::workspace::Rect> = root.rects().into_iter().collect();
        // 境界のドラッグが終わったら印を消す
        if self.resizing && !cx.has_active_drag() {
            self.resizing = false;
        }
        let panes = self.render_node(&root, Vec::new(), focused_group, &rects, window, cx);
        let picker = self.picker.as_ref().map(|(p, _, _)| p.clone());

        div()
            .id("wimux")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::new_workspace))
            .on_action(cx.listener(Self::next_workspace))
            .on_action(cx.listener(Self::prev_workspace))
            .on_action(cx.listener(Self::rename_workspace))
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::close_pane))
            .on_action(cx.listener(Self::split_right))
            .on_action(cx.listener(Self::split_down))
            .on_action(cx.listener(Self::focus_left))
            .on_action(cx.listener(Self::focus_right))
            .on_action(cx.listener(Self::focus_up))
            .on_action(cx.listener(Self::focus_down))
            .on_action(cx.listener(Self::resize_left))
            .on_action(cx.listener(Self::resize_right))
            .on_action(cx.listener(Self::resize_up))
            .on_action(cx.listener(Self::resize_down))
            .on_action(cx.listener(Self::select_ws_1))
            .on_action(cx.listener(Self::select_ws_2))
            .on_action(cx.listener(Self::select_ws_3))
            .on_action(cx.listener(Self::select_ws_4))
            .on_action(cx.listener(Self::select_ws_5))
            .on_action(cx.listener(Self::select_ws_6))
            .on_action(cx.listener(Self::select_ws_7))
            .on_action(cx.listener(Self::select_ws_8))
            .on_action(cx.listener(Self::select_ws_9))
            .on_action(cx.listener(Self::open_dir_picker))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::open_browser))
            .on_action(cx.listener(Self::jump_to_attention))
            .on_action(cx.listener(Self::split_browser))
            .on_action(cx.listener(Self::command_palette))
            .relative()
            .flex()
            .flex_row()
            .size_full()
            // 透過中は各部分（サイドバー・タブの行・区画）が透明度つきで塗る（重ね塗りで濃くならないよう）
            .when(!theme::translucent(), |el| el.bg(rgb(th().chrome_bg)))
            .text_color(rgb(th().text))
            .text_size(px(UI_TEXT))
            .when(self.sidebar_visible, |el| el.child(self.render_sidebar(cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(div().flex_1().min_h_0().child(panes)),
            )
            .when(self.settings_open, |el| el.child(self.render_settings(cx)))
            .when_some(self.ws_menu, |el, (index, pos)| {
                el.child(self.render_ws_menu(index, pos, cx))
            })
            .when_some(picker, |el, picker| {
                el.child(
                    div()
                        .id("picker-overlay")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .justify_center()
                        .pt(px(72.))
                        .bg(hsla(0., 0., 0., 0.45))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.picker = None;
                                this.focus_active(window, cx);
                            }),
                        )
                        .child(
                            div()
                                .id("picker-box")
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(picker),
                        ),
                )
            })
    }
}

/// ホームフォルダを ~ に置き換えた短い表示
fn short_path(path: &str) -> String {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    if !home.is_empty() {
        let lower_path = path.to_lowercase();
        let lower_home = home.to_lowercase();
        if lower_path == lower_home {
            return "~".into();
        }
        if lower_path.starts_with(&lower_home)
            && matches!(path.as_bytes().get(home.len()), Some(b'\\') | Some(b'/'))
        {
            return format!("~{}", &path[home.len()..]);
        }
    }
    path.to_string()
}

/// cwd から上にたどって .git を探し、現在のブランチ名を返す
fn git_branch(cwd: &str) -> Option<String> {
    let mut dir = Some(std::path::Path::new(cwd));
    while let Some(d) = dir {
        let git = d.join(".git");
        let head_path = if git.is_dir() {
            git.join("HEAD")
        } else if git.is_file() {
            // worktree / submodule: 「gitdir: <path>」
            let text = std::fs::read_to_string(&git).ok()?;
            let gitdir = text.trim().strip_prefix("gitdir:")?.trim();
            let gitdir = std::path::Path::new(gitdir);
            let gitdir = if gitdir.is_absolute() {
                gitdir.to_path_buf()
            } else {
                d.join(gitdir)
            };
            gitdir.join("HEAD")
        } else {
            dir = d.parent();
            continue;
        };
        let head = std::fs::read_to_string(head_path).ok()?;
        let head = head.trim();
        return Some(match head.strip_prefix("ref: refs/heads/") {
            Some(branch) => branch.to_string(),
            None => head.chars().take(7).collect(),
        });
    }
    None
}

/// 独自タイトルバー（OS のタイトルバーを消して、タブの行に統合する）を使うか
/// ウィンドウの背景の透過・ぼかしを設定に合わせる
fn apply_backdrop(s: &Settings, window: &Window) {
    let appearance = if s.opacity >= 0.999 {
        WindowBackgroundAppearance::Opaque
    } else {
        match s.backdrop {
            Backdrop::Transparent => WindowBackgroundAppearance::Transparent,
            Backdrop::Blur => WindowBackgroundAppearance::Blurred,
            Backdrop::Mica => WindowBackgroundAppearance::MicaBackdrop,
            Backdrop::MicaAlt => WindowBackgroundAppearance::MicaAltBackdrop,
        }
    };
    window.set_background_appearance(appearance);
}

/// 並べ替え（`from` 番目を `to` 番目へ移動）の後に、もとの `index` 番目が何番目になるか
fn moved_index(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < index && index <= to {
        index - 1
    } else if to <= index && index < from {
        index + 1
    } else {
        index
    }
}

pub fn custom_titlebar() -> bool {
    cfg!(windows) || std::env::var("WIMUX_CUSTOM_TITLEBAR").is_ok_and(|v| v == "1")
}

/// パスの末尾のフォルダ名
fn folder_name(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches(['\\', '/']);
    if trimmed.is_empty() {
        return None;
    }
    // Windows 形式（\）と Unix 形式（/）のどちらでも末尾を取る
    let name = trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed);
    Some(if name.is_empty() || name.ends_with(':') {
        format!("{trimmed}\\")
    } else {
        name.to_string()
    })
}

/// `WIMUX_RESTORE_CLAUDE=0` で Claude Code の自動再開を止められる
fn restore_claude_enabled() -> bool {
    std::env::var("WIMUX_RESTORE_CLAUDE").map_or(true, |v| v != "0")
}

#[cfg(test)]
mod tests {
    use super::{AgentState, claude_state_from_title, folder_name, moved_index};

    #[test]
    fn reorder_keeps_active() {
        // [a b c d] で 0 → 2: [b c a d]
        assert_eq!(moved_index(0, 0, 2), 2);
        assert_eq!(moved_index(1, 0, 2), 0);
        assert_eq!(moved_index(2, 0, 2), 1);
        assert_eq!(moved_index(3, 0, 2), 3);
        // 3 → 1: [a d b c]
        assert_eq!(moved_index(3, 3, 1), 1);
        assert_eq!(moved_index(1, 3, 1), 2);
        assert_eq!(moved_index(0, 3, 1), 0);
    }

    #[test]
    fn claude_title_detection() {
        assert_eq!(
            claude_state_from_title("✳ Claude Code"),
            Some(AgentState::Idle)
        );
        assert_eq!(
            claude_state_from_title("⠂ Claude Code"),
            Some(AgentState::Working)
        );
        assert_eq!(
            claude_state_from_title("C:\\Users\\kento - PowerShell"),
            None
        );
    }

    #[test]
    fn folder_names() {
        assert_eq!(
            folder_name("C:\\Users\\kento\\wimux").as_deref(),
            Some("wimux")
        );
        assert_eq!(folder_name("/home/a/b/").as_deref(), Some("b"));
    }
}

/// v1 の分割ツリーからペインを順に取り出す
fn flatten_panes(node: NodeState) -> Vec<PaneState> {
    match node {
        NodeState::Pane(p) => vec![p],
        NodeState::Group { tabs, .. } => tabs,
        NodeState::Split { first, second, .. } => {
            let mut v = flatten_panes(*first);
            v.extend(flatten_panes(*second));
            v
        }
    }
}

/// 区画がウィンドウの端に接しているか
#[derive(Clone, Copy)]
struct Edge {
    top: bool,
    top_right: bool,
}

impl Edge {
    fn of(rect: Option<&crate::workspace::Rect>) -> Self {
        const EPS: f32 = 1e-4;
        match rect {
            Some(r) => {
                let top = r.y < EPS;
                Self {
                    top,
                    top_right: top && (r.x + r.w) > 1. - EPS,
                }
            }
            None => Self {
                top: false,
                top_right: false,
            },
        }
    }
}
