use gpui_kit::base::{
    Progress as BaseProgress, ProgressIndicator, ProgressTrack, StyledExt, h_flex,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::label::Label;
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::{
    App, AppContext, Axis, Context, Entity, Hsla, IntoElement, ParentElement, SharedString, Styled,
    Subscription, Window, div, hsla, prelude::FluentBuilder as _, px, relative,
};
use mclib::java::java_version::JavaVersion;
use mclib::settings::GameWindowSize;
use std::time::Duration;
use sysinfo::System;

use crate::data::settings::AppSettings;

/// 系统的物理内存上限（M），取不到时退回 32G。
///
/// 结果缓存起来：这个值在一次运行里不会变，而滑块每帧都要用它。
fn max_memory_mb() -> usize {
    use std::sync::OnceLock;

    static MAX_MEMORY_MB: OnceLock<usize> = OnceLock::new();
    *MAX_MEMORY_MB.get_or_init(|| {
        let total_mb = (System::new_all().total_memory() / 1024 / 1024) as usize;
        if total_mb > 0 { total_mb } else { 32 * 1024 }
    })
}

/// 当前系统已用内存（M）。每次都要重新采样，所以不走缓存。
fn used_memory_mb() -> usize {
    (System::new_all().used_memory() / 1024 / 1024) as usize
}

/// 把已选中的 Java 版本转换成下拉框的取值（存的是 java 可执行文件路径）。
fn java_value(java: Option<&JavaVersion>) -> SharedString {
    match java {
        Some(java) => SharedString::from(java.path_buf.to_string_lossy().to_string()),
        None => SharedString::from("auto"),
    }
}

/// 把 `GlobalSettings::memory` 换算成滑块上的数值（M）。
///
/// 下限必须和滑块的 `.min()` 一致，否则塞进滑块的值会被它内部的
/// `clamp(min, max)` 反过来卡住。
fn memory_value(memory: Option<usize>) -> usize {
    memory.unwrap_or(4 * 1024).clamp(512, max_memory_mb())
}

/// 窗口尺寸下拉框的取值 → 显示文本。
fn window_size_option(size: &GameWindowSize) -> (&'static str, &'static str) {
    match size {
        GameWindowSize::Windowed(_, _) => ("windowed", "窗口化"),
        GameWindowSize::Minimum => ("min", "最小化"),
        GameWindowSize::Maximum => ("max", "最大化"),
        GameWindowSize::Fullscreen => ("fullscreen", "全屏"),
    }
}

/// 把窗口尺寸的当前长宽取出来，非窗口化时给个默认值。
fn windowed_size(size: &GameWindowSize) -> (usize, usize) {
    match size {
        GameWindowSize::Windowed(w, h) => (*w, *h),
        _ => (860, 640),
    }
}

/// 写回窗口尺寸并落盘。切换到窗口化时保留原来的长宽。
fn set_window_size(size: GameWindowSize, cx: &mut App) {
    cx.global_mut::<AppSettings>().global_settings.window_size = size;
    let _ = cx.global::<AppSettings>().save();
    cx.refresh_windows();
}

/// 一个只收数字的长/宽输入框，值直接写回窗口尺寸。
///
/// `pick` 从窗口尺寸里取当前值，`set` 把新值合回去，这样宽和高能共用同一段代码。
struct SizeInput {
    input: Entity<InputState>,
    _subscription: Subscription,
}

fn size_input(
    name: &'static str,
    placeholder: &'static str,
    size: GameWindowSize,
    pick: fn(&GameWindowSize) -> usize,
    set: fn(GameWindowSize, usize) -> GameWindowSize,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    let value = pick(&size);
    let state = window.use_keyed_state(name, cx, move |window, cx| {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value.to_string())
                .placeholder(placeholder)
                .validate(|text, _| text.len() <= 4 && text.chars().all(|c| c.is_ascii_digit()))
        });
        let _subscription = cx.subscribe_in(
            &input,
            window,
            move |_: &mut SizeInput,
                  input: &Entity<InputState>,
                  event: &InputEvent,
                  _window: &mut Window,
                  cx: &mut Context<SizeInput>| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                // 输入框里可能是空的（正在删改），这时沿用原值，不写 0 进去。
                let Some(parsed) = input.read(cx).value().parse::<usize>().ok() else {
                    return;
                };
                let current = cx
                    .global::<AppSettings>()
                    .global_settings
                    .window_size
                    .clone();
                set_window_size(set(current, parsed), cx);
            },
        );

        SizeInput {
            input,
            _subscription,
        }
    });

    let input = state.read(cx).input.clone();
    // 外部改过尺寸（比如切换了模式）时把输入框对齐。
    let current = input.read(cx).value().to_string();
    if current != value.to_string() {
        input.update(cx, |input, cx| {
            input.set_value(SharedString::from(value.to_string()), window, cx)
        });
    }

    input
}

pub fn global_game_settings_group<U: 'static>(cx: &mut Context<U>) -> SettingGroup {
    let app_settings = cx.global::<AppSettings>();

    // 用 java 可执行文件路径作为下拉框的取值：同一个版本号可能装了多份，路径才是唯一的。
    // 同时把版本号作为显示文本，方便阅读。
    let mut java_options = app_settings
        .java_versions
        .iter()
        .map(|java| {
            (
                SharedString::from(java.path_buf.to_string_lossy().to_string()),
                SharedString::from(java.version.clone()),
            )
        })
        .collect::<Vec<_>>();
    java_options.push((SharedString::from("auto"), SharedString::from("自动选择")));

    let max_memory_mb = max_memory_mb();
    let memory_mb = memory_value(app_settings.global_settings.memory);
    // 自动分配时滑块禁用。这里在渲染期快照一次即可：切换开关时会 refresh_windows() 重渲染。
    let auto_memory = app_settings.global_settings.memory.is_none();

    SettingGroup::new()
        .title("通用设置")
        .item(
            SettingItem::new(
                "Java 版本选择",
                SettingField::dropdown(
                    java_options,
                    |cx: &App| java_value(cx.global::<AppSettings>().global_settings.java.as_ref()),
                    |value: SharedString, cx: &mut App| {
                        let selected = if value.as_ref() == "auto" {
                            None
                        } else {
                            cx.global::<AppSettings>()
                                .java_versions
                                .iter()
                                .find(|java| java.path_buf.to_string_lossy() == value.as_ref())
                                .cloned()
                        };

                        cx.global_mut::<AppSettings>().global_settings.java = selected;
                        let _ = cx.global::<AppSettings>().save();

                        // 让设置界面重新渲染，下拉框按钮上的文字才会立刻跟着变。
                        cx.refresh_windows();
                    },
                ),
            )
            .description("不指定时由启动器根据游戏版本自动选择"),
        )
        .item(
            SettingItem::new(
                "自动分配内存",
                SettingField::switch(
                    |cx: &App| cx.global::<AppSettings>().global_settings.memory.is_none(),
                    |auto: bool, cx: &mut App| {
                        let settings = cx.global_mut::<AppSettings>();
                        settings.global_settings.memory =
                            if auto { None } else { Some(memory_value(None)) };
                        let _ = cx.global::<AppSettings>().save();

                        // 自动 / 手动切换后，滑块的可用状态和位置都要跟着变。
                        cx.refresh_windows();
                    },
                ),
            )
            .description("开启后由启动器按机器情况自动决定，下面的滑块会被禁用"),
        )
        .item(
            SettingItem::new(
                "分配内存",
                SettingField::render(move |_options, window, cx| {
                    // 滑块状态放在 window 的 keyed state 里：跨帧保留，也不会在每次重渲染时被重置。
                    let state = window.use_keyed_state("memory-slider", cx, move |_window, cx| {
                        let slider = cx.new(|_| {
                            SliderState::new()
                                .max(max_memory_mb as f32) // 难崩之要先改最大再改最小不然会panic
                                .min(512.0)
                                .step(512.0)
                                .default_value(memory_mb as f32)
                        });
                        let subscription = cx.subscribe(
                            &slider,
                            move |_, slider: Entity<SliderState>, event: &SliderEvent, cx| {
                                let mb = slider.read(cx).value().end() as usize;

                                cx.global_mut::<AppSettings>().global_settings.memory = Some(mb);
                                if matches!(event, SliderEvent::Release(_)) {
                                    let _ = cx.global::<AppSettings>().save();
                                }
                            },
                        );

                        (slider, subscription)
                    });

                    let slider = state.read(cx).0.clone();

                    // 设置值被外部改动时（比如关掉「自动分配内存」会重置成默认值），
                    // 把滑块拉回来对齐，避免滑块位置和实际生效的值不一致。
                    let target_mb = memory_value(cx.global::<AppSettings>().global_settings.memory);
                    let value = slider.read(cx).value().end() as usize;
                    if value != target_mb {
                        slider.update(cx, |state, cx| {
                            state.set_value(target_mb as f32, window, cx)
                        });
                    }
                    let value = slider.read(cx).value().end() as usize;

                    // 每 1 秒重新采样一次系统内存并重绘。放在 keyed state 里保证只起一个循环，
                    // 重渲染不会重复开任务；窗口关掉后 update_in 失败，循环自己结束。
                    window.use_keyed_state("memory-usage-tick", cx, move |window, cx| {
                        cx.spawn_in(window, async move |view, cx| {
                            loop {
                                cx.background_executor().timer(Duration::from_secs(1)).await;
                                let alive = view.update_in(cx, |_, _, cx| cx.notify()).is_ok();
                                if !alive {
                                    break;
                                }
                            }
                        })
                        .detach();
                    });

                    // 轨道分两段：深暖色是系统已用，浅暖色是本次打算分配的量；
                    // 右边只显示分配之后还剩多少可用内存。
                    let used_mb = used_memory_mb();
                    let total_mb = max_memory_mb;
                    let remaining_mb = total_mb.saturating_sub(used_mb.saturating_add(value));

                    div()
                        .w_full()
                        .v_flex()
                        .gap_2()
                        .child(
                            h_flex()
                                .w_full()
                                .gap_3()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .child(Slider::new(&slider).disabled(auto_memory)),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .w_20()
                                        .child(Label::new(format!("{} M", value))),
                                ),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .gap_3()
                                .items_center()
                                .child(div().flex_1().child(memory_bar(
                                    "memory-usage",
                                    used_mb,
                                    value,
                                    total_mb,
                                    // 已用：深橙红；分配：亮黄橙；分界线：主题底色
                                    hsla(18.0, 0.85, 0.48, 1.0),
                                    hsla(40.0, 0.95, 0.72, 1.0),
                                    cx.theme().background,
                                )))
                                .child(
                                    div().flex_none().w_20().child(
                                        Label::new(format!(
                                            "剩余 {:.1} G",
                                            remaining_mb as f32 / 1024.0
                                        ))
                                        .text_sm(),
                                    ),
                                ),
                        )
                        .into_any_element()
                }),
            )
            // 竖向布局：标题/描述在上，滑块和进度条各自占下面一行铺满宽度。
            .layout(Axis::Vertical)
            .description("拖动滑块调整，最大到系统的物理内存"),
        )
        .item(
            SettingItem::new(
                "窗口大小",
                SettingField::dropdown(
                    vec![
                        (SharedString::from("windowed"), SharedString::from("窗口化")),
                        (SharedString::from("min"), SharedString::from("最小化")),
                        (SharedString::from("max"), SharedString::from("最大化")),
                        (SharedString::from("fullscreen"), SharedString::from("全屏")),
                    ],
                    |cx: &App| {
                        SharedString::from(
                            window_size_option(
                                &cx.global::<AppSettings>().global_settings.window_size,
                            )
                            .0,
                        )
                    },
                    |value: SharedString, cx: &mut App| {
                        // 切回窗口化时沿用已保存的长宽，避免把用户填的数字冲掉。
                        let (width, height) =
                            windowed_size(&cx.global::<AppSettings>().global_settings.window_size);
                        let size = match value.as_ref() {
                            "min" => GameWindowSize::Minimum,
                            "max" => GameWindowSize::Maximum,
                            "fullscreen" => GameWindowSize::Fullscreen,
                            _ => GameWindowSize::Windowed(width, height),
                        };
                        set_window_size(size, cx);
                    },
                ),
            )
            .description("窗口化时可以自定义长宽，其余模式由游戏自己决定"),
        )
        .item(
            SettingItem::new(
                "窗口长宽",
                SettingField::render(move |_options, window, cx| {
                    let size = cx
                        .global::<AppSettings>()
                        .global_settings
                        .window_size
                        .clone();
                    let windowed = matches!(size, GameWindowSize::Windowed(_, _));

                    let width_input = size_input(
                        "window-width",
                        "宽度",
                        size.clone(),
                        |size| windowed_size(size).0,
                        |size, value| {
                            let (_, height) = windowed_size(&size);
                            GameWindowSize::Windowed(value, height)
                        },
                        window,
                        cx,
                    );
                    let height_input = size_input(
                        "window-height",
                        "高度",
                        size.clone(),
                        |size| windowed_size(size).1,
                        |size, value| {
                            let (width, _) = windowed_size(&size);
                            GameWindowSize::Windowed(width, value)
                        },
                        window,
                        cx,
                    );

                    // 两个输入框固定宽度并排靠左，不跟着设置面板宽度拉伸。
                    // min_w_0 让这一行在窄面板下能收缩，避免第二个框顶出边界。
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .flex_none()
                                .child(Input::new(&width_input).w_48().disabled(!windowed)),
                        )
                        .child(
                            div()
                                .flex_none()
                                .child(Input::new(&height_input).w_48().disabled(!windowed)),
                        )
                        .into_any_element()
                }),
            )
            .description("单位是像素，仅在窗口化模式下生效"),
        )
}

/// 一条轨道内分两段的内存条：前面是已用，紧接着是本次打算分配的量。
///
/// 两段都按 `total_mb`（系统总内存）换算，所以它们在轨道上的位置和长度可以直接比较，
/// 段与段之间的空白就是剩余可用内存。
fn memory_bar(
    id: &'static str,
    used_mb: usize,
    alloc_mb: usize,
    total_mb: usize,
    used_color: Hsla,
    alloc_color: Hsla,
    divider_color: Hsla,
) -> impl IntoElement {
    let ratio = |mb: usize| -> f32 {
        if total_mb == 0 {
            0.0
        } else {
            (mb.min(total_mb) as f32 / total_mb as f32).clamp(0.0, 1.0)
        }
    };

    let used_ratio = ratio(used_mb);
    // 分配段从已用段结束的地方开始，两段合起来最多铺满整条轨道。
    let alloc_ratio = ratio(used_mb.saturating_add(alloc_mb)) - used_ratio;

    let radius = px(4.);
    // 只在整条轨道的两个端点做圆角，两段交界处保持直角，
    // 否则铺满时两个圆角会叠在一起。
    let used_rounds_right = alloc_ratio <= 0.0;
    let alloc_rounds_left = used_ratio <= 0.0;

    BaseProgress::new(id)
        .value((used_ratio + alloc_ratio) * 100.0)
        .w_full()
        .relative()
        .h(px(8.))
        .rounded(radius)
        .child(
            ProgressTrack::new()
                .absolute()
                .size_full()
                .rounded(radius)
                .bg(used_color.opacity(0.15)),
        )
        .child(
            ProgressIndicator::new()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(relative(used_ratio))
                .rounded_l(radius)
                .when(used_rounds_right, |this| this.rounded_r(radius))
                .bg(used_color),
        )
        .child(
            ProgressIndicator::new()
                .absolute()
                .top_0()
                .left(relative(used_ratio))
                .h_full()
                .w(relative(alloc_ratio))
                // 交界处留一条背景色细线，把两段分开，不靠颜色本身去区分。
                .when(!alloc_rounds_left, |this| {
                    this.border_l_1().border_color(divider_color)
                })
                .when(alloc_rounds_left, |this| this.rounded_l(radius))
                .rounded_r(radius)
                .bg(alloc_color),
        )
}
