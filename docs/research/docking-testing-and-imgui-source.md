# User-supplied docking research source

Received on 2026-11-16. The two messages below are preserved verbatim at the
user's request so later sessions can recover the proposed investigation. They
are supplied research material, not verified Dockspace contracts. Favicon
placeholders and source labels belong to the original messages.

See [the research brief](docking-testing-and-imgui.md) for verified primary
sources, current design differences and the bounded follow-up.

## Original message 1

你可以继续修复，然后对于我们一直找一直有问题的情况 我问了下chatgpt，他说**正常，而且某种程度上，这是你随机测试真正开始发挥价值的表现。**

但我觉得你现在遇到的已经不只是「如何多发现 Bug」，而是另一个更重要的问题：

> **对于 Docking / MultiView 这样状态组合几乎无限的系统，怎样判断测试已经足够充分？什么时候应该停止追逐新的随机 Bug？**

这两个问题比单纯增加测试数量更值得解决。

我看了一下你现在的 [Dockspace 仓库](https://github.com/Latias94/dockspace?utm_source=chatgpt.com)。它已经不只是一个简单的 Split Tree，而是涉及：

- Tabs、Split、Floating Group、Docking Drag。
- 多 Surface 与 Native Window 的生命周期。
- Focus、Capture、Hover、Hit Testing。
- 布局版本、动画、输入事件的先后顺序。
- 渲染结果的 Presented / Dropped 确认。
- 异步关闭、取消、恢复以及资源所有权。

你甚至已经在测试文档中考虑了失去鼠标释放事件、旧输出迟到、取消后重新操作、窗口销毁、DPI 变化等情况，并且有确定性的事件交错测试。例如 Native Pending Release 测试就枚举了 600 种组合。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub+1



所以我不建议你简单地「再增加十倍随机种子」。你的测试基础已经相当广泛，接下来更重要的是**改变测试策略，把发现 Bug、归纳 Bug、验证修复和衡量可靠性分开**。

## **一、为什么你会感觉 Bug 永远找不完？**

这是复杂状态机的典型现象。

假设你的系统只有 12 种交互操作，测试连续执行 30 步，理论上就有：

\\[ 12^{30}\approx 2.37\times10^{32} \\]

种操作序列。当然，大量序列不合法或等价，实际可达状态远少于此，但也足以说明：**靠随机枚举覆盖所有可能性是不现实的。**

而且复杂 UI 的 Bug 往往不是单个操作引起的。

例如：

```
创建 Window B
  → 开始从 A 拖动 Pane
  → 鼠标进入 B
  → B 的 Presentation 尚未确认
  → A 收到 Capture Lost
  → B 的旧 Output 被 Dropped
  → 再次开始拖动
  → 某个输入状态永久卡住
```

每个操作单独测试可能都是正确的，问题却出在它们的先后关系上。

这里存在三种完全不同的「不断发现新问题」：

| 现象                      | 应该如何理解                       |
| ----------------------- | ---------------------------- |
| 不同种子发现同一个根因的不同表现        | 正常，但需要去重和缩小案例                |
| 修复一个状态转移后，测试进入之前到不了的新状态 | 正常，说明可探索范围扩大了                |
| 每修复一个 Bug，就破坏另一类已有行为    | 需要警惕，可能是状态所有权、事件时序或系统不变量没有统一 |

**前两种是测试有效的迹象；第三种则可能意味着架构层面的问题。**

尤其是对你这种无头库，如果有多个组件分别维护一部分交互状态，却没有明确的失效和提交边界，随机测试可能会反复找到看似不同、根因却相近的问题。

## **二、我最建议你做的改变：从随机行为转向模型测试**

你现在的做法大致是：

```
生成一个随机种子
       ↓
生成操作序列
       ↓
运行 Dockspace
       ↓
检查是否 Panic / 卡住 / 状态异常
       ↓
保存失败 Seed
```

这个思路没有问题。

但如果想进一步提高测试效率，我推荐研究 **Model-Based Testing（基于模型的测试）**。

它和普通随机测试的区别在于：除了测试真实系统，还维护一个独立、简化的参考模型。

**有状态行为生成器**

Open / Close / Drag / Resize / Present / Drop / Cancel

**Reference Model**

简单、独立的行为规则，不关心具体实现

**Dockspace**

真实布局树、交互状态机、Runtime

**Invariant / Postcondition Checker**

比较语义结果、所有权、结构约束及状态转移

例如，假设某个 Pane 被移到另一个 Split Group。

参考模型只需要知道：这个 Pane 原来属于 A，现在属于 B，其他 Pane 的成员关系应该保持不变。

真实 Dockspace 则需要完成布局树调整、节点清理、Focus 更新、Hit Region 更新等复杂工作。

只要二者的**可观察语义结果**不一致，就可能找到了问题。

注意：参考模型必须独立于被测算法，不能复制 Dockspace 的核心实现，否则双方可能犯同一个错误。

Rust 已经有现成的 [proptest-state-machine](https://github.com/proptest-rs/proptest/blob/main/book/src/proptest/state-machine.md?utm_source=chatgpt.com)，支持生成状态转移序列、比较参考模型和真实系统，并自动缩小失败序列。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub+1



但结合你现有项目规模，我不建议一开始就把所有 Native 生命周期都建成第二套系统。可以先从 Tabs、Split、Pane 所有权这些最基本的语义开始。

## **三、比随机次数更重要的：不变量（Invariant）**

不变量是指无论用户做过什么操作，系统始终必须满足的规则。

我认为 Dockspace 应该重点建立以下检查：

| 类别       | 推荐的不变量                          |
| -------- | ------------------------------- |
| 布局拓扑     | 已提交的布局树无环、节点引用有效、成员归属不重复        |
| Pane 所有权 | 每个仍存在的 Pane 都有可解释的归属，包括合法的待迁移状态 |
| 约束       | 目标几何满足有效尺寸约束，除非显式支持退化布局         |
| 原子提交     | 失败或丢弃 Candidate 不修改已提交的语义状态     |
| 输入权威     | 未确认呈现的 Hit Region 不能接受输入        |
| 取消       | 可恢复的 Cancel 不遗留捕获，也不阻止下一次合法操作   |
| 生命周期     | 窗口销毁、资源释放不会让迟到的 Callback 重新获得权限 |
| 持久化      | 序列化、恢复后布局语义等价，Runtime 临时状态不需要相同 |

需要强调，这些检查不是所有时候都应该立即成立。

例如动画中两个 Pane 可以临时重叠，因此不能要求 *Presentation Rect* 始终是无重叠的 Split Partition。

同理，一个 Native Transfer 可能正在等待精确的 Presentation 确认，因此不能要求每次 `PointerUp` 以后所有迁移立即完成。

你仓库的 ADR 已经非常明确地区分了语义提交、输出呈现、Receiver Authority 和等待中的 Native Mutation。这些恰恰应该成为不同测试检查点的依据。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub+1



### **特别注意：「卡住」不一定就是 Bug**

你提到随机测试能找到一些「走到没考虑到的状态，然后卡住」的情况。

这里建议细分：

**真正的卡死：** 输入已经结束，所有必要的外部结果都已提供，系统仍无法完成原本应该可以完成的操作。

**合法等待：** 系统等待尚未到达的 Presentation、Native Effect 或其他外部确认。

**明确拒绝：** 测试生成器给出了违反 API 契约的原始事件，系统按照规定进入不可继续的失败状态。

第三种在 Dockspace 尤其重要，因为你的文档已经定义了某些 contradictory raw ingress 会导致永久拒绝，而非可恢复 Cancel。这样的情况不能简单算成「应该还能继续拖拽」的 Bug。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



## **四、把失败 Seed 变成最小复现，比继续找新 Seed 更重要**

我认为这是你目前最值得投入的改进。

假设随机执行 300 次交互以后失败：

```
Seed: 817231
Steps: 300
Failure: stale receiver remained active
```

这虽然可复现，但还是太复杂。

理想的测试工具应当能够把它缩减为：

```
1. BeginDrag(PaneA, Window1)
2. ChangeScale(Window1, 1.5)
3. ReportPresentation(Dropped)
4. EndDrag()
5. TryInteractionUsingOldReceiver()
```

缩减后的 5 步，比原来 300 步更有价值。

这叫 **Shrinking / Delta Debugging**。

它通常需要两层缩减：

第一层尝试删除整段不相关的操作；第二层尝试简化参数，例如减少 Pane 数量、简化布局树、降低窗口数量，或者将复杂坐标替换成固定的中心点、边缘点。

不过，对有状态操作序列不能简单删除而不检查有效性。例如删除了 `CreatePane`，后面的 `DragPane` 可能就失效了。缩减器必须重新解析逻辑 ID、检查前置条件，必要时修复依赖关系。

这也是 `proptest-state-machine` 相较普通随机生成器很有价值的地方。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



我还建议保存完整 Action Trace，而不只是 Seed：

```
{
  "seed": 817231,
  "failure": "stale_receiver",
  "actions": [
    {"type": "begin_drag", "pane": "A"},
    {"type": "change_scale", "scale": 1.5},
    {"type": "presentation", "result": "dropped"},
    {"type": "end_drag"},
    {"type": "try_old_receiver"}
  ]
}
```

这样即使未来修改了随机数生成算法、操作权重或者新增操作类型，仍然能够准确重放历史故障。

此外，使用符号 Pane ID、逻辑对象选择器，而不是依赖运行时生成的原始 ID，会使 Shrinking 更可靠。

## **五、从「随机覆盖」升级为「状态覆盖」**

还有一个问题：随机操作序列不一定能探索到真正有价值的状态。

例如随机执行 10000 次 `SelectTab`、`MovePane`、`Resize`，也不一定能命中一次「窗口已创建，但 First-live Presentation 尚未确认」的状态。

我建议让生成器识别和定向生成这些状态：

| 状态覆盖目标                  | 重点测试                       |
| ----------------------- | -------------------------- |
| Idle                    | 空闲时重复操作、无效操作               |
| Dragging                | 拖动中目标变化、失去 Capture         |
| Preview Pending         | 预览尚未 Presented 时取消         |
| Native Creation Pending | Child 创建途中 Source 发生变化     |
| Closing Deferred        | 关闭请求等待应用批准时继续操作            |
| Output Backpressure     | 多个 Output 等待确认或被丢弃         |
| Recovering              | 失效后迟到事件与新的合法输入             |
| Geometry Revoked        | DPI、Placement、尺寸变化后旧命中权限失效 |

可以为每种状态统计覆盖次数，并记录有多少种不同的状态转移被触发。

这样，新随机种子不仅是「再跑一次」，而是尽量探索**以前没覆盖过的状态及状态转移**。

你已经有一些确定性的 Producer Interleaving 测试。我会继续保留它们，不需要全部替换为随机测试。穷举一个受约束的小事件集合，往往比随机执行几百万次更容易证明某个协议边界正确。

## **六、怎样知道测试正在收敛？**

我更建议观察下面这些指标，而不是只统计发现多少 Bug。

| 指标               | 希望看到的趋势     |
| ---------------- | ----------- |
| 每万条序列的独立失败根因数    | 逐渐下降        |
| 历史失败 Seed 的重放成功率 | 接近 100%     |
| 最小失败序列平均长度       | 下降，便于定位     |
| 有效状态转移覆盖         | 逐渐增长，最终趋于平稳 |
| 相同根因反复出现的数量      | 明显减少        |
| 修复旧 Bug 造成的新回归   | 趋近于零        |
| 关键流程的测试稳定性       | 持续提高        |

我会把每个失败案例按照**根因**归类，而不是每个 Seed 都算一个新 Bug。

例如 100 个 Seed 中有 80 个都源于同一处旧 Presentation 被错误授予 Receiver Authority，它们应该算作一个根因及其复现集合，而不是 80 个独立缺陷。

这种方式能避免测试越来越多、心理上却感觉项目越来越不可靠。

也可以建立固定的 Holdout Seeds：它们不参与日常针对性修复，只用于阶段性验证泛化能力。避免不知不觉把实现和测试都优化成只适应已有种子的行为。

## **七、我建议你实际采用的测试流程**

**阶段 A：快速回归**

每次提交执行单元测试、核心 Invariant、已知最小失败案例和小规模固定 Seed。

**阶段 B：有状态探索**

夜间运行状态引导的随机序列，并覆盖各种 Presentation、取消、窗口与输入时序。

**阶段 C：自动归纳失败**

收集完整 Action Trace → 确定性重放 → Shrink → 根据不变量和协议边界归类。

**阶段 D：真实平台验证**

对语义层已验证的关键交互，使用 egui、GPUI、X11/macOS/Windows 驱动确认真实输入和窗口行为。

你仓库已经具备 Core Host Conformance、Recording Host、egui Product Harness、GPUI Harness 和 Native Qualification 等多层测试基础，因此不需要推倒重来。可以优先为现有随机测试加入自动 Shrinking、失败分类和状态覆盖统计。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



## **最后：什么时候可以认为 Dockspace 足够稳定？**

我的标准不是「再也找不到 Bug」。

我更看重这三件事：

**第一，核心不变量不被破坏。** 无论用户如何组合操作，不能丢失 Pane、重复所有权、错误释放资源，或让过期输入获得权限。

**第二，常见操作和重要异常路径有确定性保障。** 正常 Docking、取消、窗口关闭、DPI 变化等行为有可靠的回归测试，而不是依赖随机种子碰巧覆盖。

**第三，探索的边际收益开始下降。** 不断提高测试序列的复杂度和规模时，新增的独立根因越来越少，而且高严重度问题不再出现。

如果随机测试运行很多轮仍然持续发现严重的所有权破坏、不可恢复的交互卡死，那么值得停下来审视状态模型和协议边界；如果发现的只是越来越偏僻、影响有限的新组合，则可以按严重程度管理，而不是要求修完所有潜在情况才发布。

**对你目前的 Dockspace，我最建议的不是继续提高随机测试强度，而是先解决一个问题：让每个失败的 Seed 都能自动变成一条简短、可解释、可长期保留的交互回归测试。**

这会把「似乎永远做不完的 Bug 搜索」，逐渐变成一个可以衡量、可以收敛、可以维护的质量工程。 你可以看看 有必要可以无畏重构测试

## Original message 2

你可以先修你之前在修的问题。 然后看看这些imgui的调查**不是。Dear ImGui 并不是单纯靠用户反馈，然后一个 Bug 一个 Bug 地修。它有专门的自动化测试引擎，而且 Docking / Multi-Viewport 确实在持续增加自动化测试。**

不过，你的直觉也有一部分是对的：**Dear ImGui 即使有多年的开发积累和测试基础设施，Docking / Multi-Viewport 仍然存在长期未解决的问题。**

我查到了几个很关键的事实：

1. **Dear ImGui Test Engine** 从 2018 年就在开发和使用，直到 2022 年底才公开。
2. 官方有独立的 **ImGui Test Suite**，能够模拟鼠标、键盘和 UI 操作，并检查运行时状态。
3. 官方 GitHub Actions 会构建、运行测试；Docking 分支还会执行专门的 **Viewport Mock Tests**。
4. 作者 Omar 在 2026 年更新的 Docking 文档中明确表示，Docking 的内部代码仍然过于复杂、脆弱，计划第三次重写，并正在逐步补充自动化测试来支持重构。

   [image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

   GitHub+2



最后一点特别值得关注。它说明，即使是 Dear ImGui，也没有找到一种能彻底消除 Docking 复杂性的测试方法。

它采用的是**自动化回归 + 模拟平台行为 + 真实平台验证 + 用户反馈 + 逐步调整架构**的组合。

## **一、ImGui 实际怎么测试 Docking？**

我直接读了官方的 [imgui_tests_docking.cpp](https://github.com/ocornut/imgui_test_engine/blob/main/imgui_test_suite/imgui_tests_docking.cpp?utm_source=chatgpt.com)，里面包含大量具名、可重复执行的测试场景。

例如：

| 官方测试名称                           | 验证什么                       |
| -------------------------------- | -------------------------- |
| `docking_basic_1`                | 两个窗口合并、拆开、重新 Dock，验证节点和位置  |
| `docking_tab_order`              | Tab 的排列顺序是否正确              |
| `docking_tab_order_preserve`     | 移动整个 DockNode 后 Tab 顺序是否保留 |
| `docking_tab_focus_restore`      | 切换 Tab 后焦点能否恢复             |
| `docking_undock_focus_retention` | 拖出窗口时，其他窗口出现是否错误抢走焦点       |
| `docking_sizing_1`               | 多层 Split 的尺寸和比例是否符合预期      |
| `docking_dockspace_keep_alive`   | 隐藏 Dockspace 后是否保留布局信息     |
| `docking_preserve_docking_info`  | 窗口关闭、重新出现后能否恢复 Dock 信息     |
| `docking_settings_invalid_1`     | 损坏或不一致的布局配置能否安全处理          |

这些都是真实存在于测试源码中的用例。

[image](https://www.google.com/s2/favicons?domain=https://raw.githubusercontent.com\&sz=32)

GitHub



### **一个真实的测试例子**

官方测试里有类似下面的代码，先把两个窗口 Dock 在一起，然后直接检查它们的内部节点关系：

```
ctx->DockInto("AAA", "BBB");

IM_CHECK(window_aaa->DockNode != NULL);
IM_CHECK(window_aaa->DockNode == window_bbb->DockNode);
```

后续还会把 AAA 拆出，再 Dock 回去，并验证 Dock ID、窗口位置、DockNode 尺寸等是否正确。

[image](https://www.google.com/s2/favicons?domain=https://raw.githubusercontent.com\&sz=32)

GitHub



这很有代表性：**它不是截图比较，也不只是检查有没有崩溃，而是实际执行用户交互，并检查内部语义。**

更有意思的一个例子是 `docking_undock_focus_retention`：

- Window A 和 Window B 原本停靠在一起。
- Window A 的内容会在特定状态下显示另一个 Window A2。
- 开始把 Window B 拖出去。
- 拖动中，A2 重新显示。
- 测试断言：Window B 仍然是正在移动的窗口，焦点不能被 A2 意外夺走。

这个测试对应历史问题 #3392。

[image](https://www.google.com/s2/favicons?domain=https://raw.githubusercontent.com\&sz=32)

GitHub



这类场景与你的随机行为测试发现的问题非常相似，只不过 ImGui 将它们整理成了**确定性的最小交互剧本**。

## **二、Multi-Viewport 测试更值得参考：它有 Mock Platform**

这是我觉得对 Dockspace 最有价值的发现。

官方 CI 中明确存在这样的命令：

```
./imgui_test_suite \
  -nogui -nopause -v2 -ve4 \
  -viewport-mock viewport
```

在 Windows 和 Linux 测试任务中，这套命令会在检测到 Docking 功能后执行。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



这代表可以在**不创建真实系统窗口**的测试环境中，模拟 Multi-Viewport 所需的平台行为。

在 [imgui_tests_viewports.cpp](https://github.com/ocornut/imgui_test_engine/blob/main/imgui_test_suite/imgui_tests_viewports.cpp?utm_source=chatgpt.com) 中，还能看到这些具体场景：

| 测试                          | 测试行为                                 |
| --------------------------- | ------------------------------------ |
| `viewport_basic_1`          | 窗口从主 Viewport 移出去，生成独立 Viewport，再移回来 |
| `viewport_translate`        | 主窗口发生位置变化时，子窗口坐标如何更新                 |
| `viewport_platform_focus_2` | 窗口跨 Viewport 后是否保留正确焦点               |
| `viewport_platform_focus_3` | Modal、Popup 切换时是否产生错误的平台窗口或焦点        |
| `viewport_platform_close`   | 系统关闭一个平台窗口后，关联的 ImGui 窗口是否正确关闭       |
| `viewport_platform_close_2` | 嵌套窗口与 Dockspace 在关闭时的行为              |

[image](https://www.google.com/s2/favicons?domain=https://raw.githubusercontent.com\&sz=32)

GitHub+2

其中 `viewport_basic_1` 的检查方式很直接：

```
// 移出主窗口
ctx->WindowMove(
    "",
    main_viewport->Pos - ImVec2(20, 20)
);

IM_CHECK(window->Viewport != main_viewport);
IM_CHECK(window->ViewportOwned);

// 移回主窗口
ctx->WindowMove(
    "",
    main_viewport->WorkPos + ImVec2(20, 20)
);

IM_CHECK(window->Viewport == main_viewport);
```

它验证的是 **Viewport 所有权如何随交互迁移**，而不是操作系统到底有没有显示出一个新窗口。

这种测试的优势是速度快、稳定、能够在 CI 中重复运行。

但它也有明确的边界：Mock 不等于真实 Win32、Cocoa、X11 窗口管理器。真实系统里的窗口焦点、鼠标捕获、DPI 和合成器时序，仍然需要额外测试。

## **三、即便如此，ImGui 依然存在很多问题**

这可能才是你真正想了解的。

官方源码中保留了不少未完成的测试、`FIXME-TESTS`，甚至还有通过 `IMGUI_BROKEN_TESTS` 条件宏隔离的测试。

例如在 `docking_sizing_1` 里，就有一项针对多层 Split 尺寸分配的检查被条件禁用，注释说明当前按深度优先进行尺寸分配，还不能保证特定布局得到理想的调整结果。

[image](https://www.google.com/s2/favicons?domain=https://raw.githubusercontent.com\&sz=32)

GitHub



也就是说：

**他们不是先把所有测试修到完美，才允许功能被使用。**

有些是明确接受的当前限制，有些是待修复问题，还有些是测试本身不够可靠。

而且平台问题更加明显。

Dear ImGui 官方的 Multi-Viewport 文档至今仍然写着：

- Linux/X11 在不少窗口管理器下存在问题，需要有人投入时间调查和补充测试。
- 现有 Multi-Viewport 设计不支持 Wayland，因为它依赖应用读取和设置窗口的全局位置。

  [image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

  GitHub



甚至在当前 GitHub Actions 的测试配置中，macOS 测试 Job 仍然是被注释的状态；Windows 与 Linux 则有实际启用的自动化任务。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



所以，自动化测试并没有解决所有平台复杂性。

### **为什么作者还打算重写 Docking？**

Omar 在 2026 年 1 月更新的 Wiki 中，给出的理由非常直接：

- 许多公开问题没有足够简单的修复方案。
- 现有代码的复杂性和脆弱性没有达到他期望的质量。
- 部分核心概念需要重新设计。
- 正在逐步补充自动化测试，以帮助未来重写。

而且明确说希望第三次从头重写 Docking。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



这个信息很重要。

**有时候持续发现大量边缘 Bug，不是因为测试还不够，而是底层模型本身已经难以维护。**

不过，这不等于每个持续发现 Bug 的项目都需要重构。关键在于它们是否反复指向相同的状态所有权、事件时序或架构边界问题。

## **四、ImGui 和你现在的测试方式有什么区别？**

综合公开的测试代码，可以这样比较：

| 维度     | ImGui 已公开的做法                     | 你目前描述的做法             |
| ------ | -------------------------------- | -------------------- |
| 交互驱动   | 确定性 UI 操作脚本                      | 随机行为与固定种子            |
| 状态检查   | 直接读取内部 DockNode、Focus、Viewport 等 | 已有状态检查，具体覆盖程度取决于实现   |
| 复杂场景   | 专门编写边界条件回归                       | 随机序列主动探索未知组合         |
| 平台模拟   | Viewport Mock                    | 无头 Host / Runtime 测试 |
| CI     | 执行具名回归用例                         | 可运行种子集与回归测试          |
| Bug 发现 | 回归、人工测试、用户报告                     | 随机探索、回归、人工测试         |
| 未解决问题  | 显式记录 TODO、FIXME 和已知限制            | 可以建立同类的已知限制清单        |

这张表并不代表 ImGui 没有使用任何随机测试。我目前能确认的是，它公开的 Docking / Viewport 测试主要采用具名、确定性的场景；没有找到足以确认其使用大规模状态覆盖型 Fuzzer 的官方材料。

## **五、我觉得 Dockspace 最值得借鉴的不是测试数量，而是测试组织方式**

与其完全照搬 ImGui Test Engine，不如从它的经验中提取三个实践。

**第一：随机测试负责发现未知行为，具名回归负责守住已知行为。**

你可以保留现有随机探索，但每次发现真正的新根因，都把最小操作序列整理成一项有明确名称的回归测试，例如：

`native_drag_cancel_after_stale_presentation`

然后为它写下明确的预期行为。长期积累的不是数千个无法解释的 Seed，而是一套越来越完善的交互行为规范。

**第二：把平台模拟当成正式测试能力，而不是临时 Mock。**

ImGui 的 Viewport Mock 能在 CI 中运行真实的窗口所有权、焦点与迁移测试。对于 Dockspace 这种无头库，这个方向尤其合理。

你甚至可以专门设计 Mock Window Manager，对窗口创建、焦点转移、拖放释放、DPI、平台事件延迟进行可编排的模拟。不同于纯随机输入，这还能让你指定精确的故障注入点。

**第三：接受明确的功能边界，不要试图用测试替代产品决策。**

例如 Wayland 不允许任意设置窗口全局位置，那么你就要决定：跨原生窗口 Docking 在该平台是降级支持、采用不同协议，还是明确不支持。

这是一项产品和架构决策，不是再运行一百万次测试就能解决的事情。

## **六、值得直接研究的官方源码**

[**imgui_tests_docking.cpp**](https://github.com/ocornut/imgui_test_engine/blob/main/imgui_test_suite/imgui_tests_docking.cpp?utm_source=chatgpt.com)

重点读 `docking_basic_1`、`docking_undock_focus_retention`、`docking_sizing_1`。理解如何围绕真实用户操作检查内部状态。

[**imgui_tests_viewports.cpp**](https://github.com/ocornut/imgui_test_engine/blob/main/imgui_test_suite/imgui_tests_viewports.cpp?utm_source=chatgpt.com)

重点读平台窗口的迁移、Focus、Close 测试，研究 Mock Platform 与真实窗口行为的边界。

[**imgui_te_context.h**](https://github.com/ocornut/imgui_test_engine/blob/main/imgui_test_engine/imgui_te_context.h?utm_source=chatgpt.com)

测试驱动层 API。包括 `MouseDown`、`MouseMove`、`DockInto`、`ViewportPlatform_CloseWindow` 等操作，适合参考如何设计测试 DSL。

[**ImGui 官方 CI 工作流**](https://github.com/ocornut/imgui/blob/master/.github/workflows/build.yml?utm_source=chatgpt.com)

可以直接看到官方如何在 Windows 和 Linux 上运行无界面测试及 Viewport Mock。

## **我的结论**

你问「难道就是靠反馈，一点一点修 Bug 吗？」

**一定程度上，是的。但关键差别在于，每个修复是否在降低未来出错的概率。**

ImGui 的实践不是无休止地人工修 Bug，而是把发现的问题转化为可重复的行为测试，并逐步建立一套保护现有功能的测试体系。与此同时，维护者也承认，当内部设计过于复杂时，仅仅添加回归测试还不够。

对 Dockspace 来说，你已经具备随机行为测试，这是很好的起点。**接下来我会更关注你发现的 Bug 是否开始收敛到少数几类状态模型问题。**

如果连续修复几十个 Bug，它们都指向同一个 Pending / Commit / Cancel 规则的模糊地带，那比继续增加测试更有价值的事情，是把这条规则重新设计清楚。

而如果新的 Bug 分散在不同边界，严重程度也不断下降，那反而可能说明你的系统正在逐渐成熟。

我认为 ImGui 最值得学习的，不是它有多少测试，而是它的作者**既认真利用测试积累工程确定性，也愿意承认当前架构仍然有需要推倒重来的部分**。找到了，而且有一份比 Docking Wiki 更具体、更值得研究的资料。

**Dear ImGui 作者 Omar Cornut 在《10 years of Dear ImGui》长文中，专门用第 6.4、6.5 节说明了 Docking v3 重写的动机和初步方向。**

原文：[GitHub Issue #7892 — 10 years of Dear ImGui](https://github.com/ocornut/imgui/issues/7892?utm_source=chatgpt.com)

其中有一句话非常关键：

> Probably transition toward more of a model where windows are dictating node behaviors.

意思是：**未来可能转向由 Window 决定 Dock Node 行为的模型。**

这不是普通的代码清理，而是可能涉及 Docking 的核心职责分配。

## **一、Omar 明确提出的 5 个重构方向**

| 问题                | 当前困境               | v3 初步方向                 |
| ----------------- | ------------------ | ----------------------- |
| Dock Node 身份      | 节点会分裂、合并、移动，长期识别困难 | 重新设计节点定位与身份模型           |
| DockBuilder API   | API 不完善、难以正式公开     | 重做 Builder API，理想情况下公开  |
| Central Node      | 概念含糊、包含过多特殊规则      | 拆分为多个独立属性，部分规则变为可选      |
| Window 与 Node 的职责 | Docking 改变了窗口原本的行为 | 可能让 Window 决定 Node 的行为  |
| 尺寸与行为约束           | 很难对 Dock Node 施加约束 | 让 Node Constraint 更容易表达 |

这些不是我的推测，而是作者本人公开列出的方向。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



但这里有个重要限制：**Omar 明确说过，他当时还没有确定 v3 重写的核心设计需求。** 因此它们是探索方向，不是已经敲定的技术方案。

下面逐项分析这些问题的实际表现，以及能从 Issue 中找到哪些更具体的证据。

## **二、问题 1：Dock Node 的身份无法长期稳定**

架构核心问题

[Issue #5161 · Long-term references to dock node identifiers](https://github.com/ocornut/imgui/issues/5161?utm_source=chatgpt.com)

我认为这是所有线索中最重要的一条。

### **具体发生了什么？**

假设应用初始创建三个区域：

**Node A**

Hierarchy

**Node B**

Scene View

用户把 Node A 再次分割

Node A1

Node A2

**Node B**

概念示意：A 分割后，原来的标识不再明确代表哪个实际停靠位置。

应用原本保存了 Node A 的 ID，以便以后将某个窗口放到左侧。

但 A 分裂成两个子节点后，A 可能成为非叶节点，不能再直接作为停靠目标。

Omar 在 #5161 中解释：如果允许任意 Split / Merge，某些情况下就不可能无歧义地追踪节点的长期身份。

他甚至考虑过让 Node 保存多个自定义 Tag，以应对节点合并。然而这种方法依然有问题：即使一个节点保留 `left-panel` 标签，它也不一定还在左侧，更不一定包含原先的窗口。

### **作者提出了什么？**

**不要过度依赖长期保存的 Node ID，而是动态查找符合用户意图的位置。**

比如用户希望把一个新窗口放到右侧，可以在当前布局里查找：

- 最右边的有效叶子节点；
- 如果存在多个候选，选择最大的；
- 或者选择最近使用过的节点。

作者在评论里明确提出过这个方向，甚至表示未来可以提供相应的 Helper。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



这是一个很深的架构问题：

**布局节点的身份（Node Identity）和停靠位置的语义（Docking Intent）不应该被当作同一回事。**

对于 Dockspace，我认为应该特别重视这个区别。稳定的 `PaneId` 很有价值，但不意味着 `DockNodeId` 在任意 Split / Merge 后仍能永久表达原来的语义位置。

## **三、问题 2：Central Node 把几种不相关的职责绑在一起**

相关讨论：

[#3168 · Trying to avoid an empty central node](https://github.com/ocornut/imgui/issues/3168?utm_source=chatgpt.com)[#5209 · How to keep all the dockspace ratios the same when resizing?](https://github.com/ocornut/imgui/issues/5209?utm_source=chatgpt.com)

ImGui 的 Central Node 不是一个普通的中心面板。

根据官方 Glossary，它同时具有以下语义：

1. 即使没有窗口也保持存在。
2. 优先使用其他节点分配后剩余的空间。
3. 可以通过 `PassthruCentralNode` 实现透明和输入穿透。

这些特性原本服务于典型的游戏编辑器：中央是 Scene View，四周是工具面板。

但问题是，不是每个应用都有天然的中心区域。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



### **一个真实的尺寸分配问题**

在 #5209 中，用户构建了 Hierarchy、Inspector、View、Asset Browser 布局。

初始显示正常，但改变窗口大小时，Inspector 所在区域被不符合预期地拉伸。

Omar 指出，问题在于用户的 `DockBuilderSplitNode()` 调用方向，使 Inspector 所在节点意外继承了 Central Node 的剩余空间行为。

改变 Split 的方向，让 View 继承 Central Node 属性，就能获得更符合预期的尺寸变化。

**同样的窗口组合，因为 Split 构建方向不同，最终会拥有不同的尺寸分配语义。**

这显然不够直观。

### **v3 的设计方向**

Omar 提出：

- 允许完全禁用 Central Node。
- 把它拆成更具体的独立属性。
- 不再要求所有行为都绑定到一个特殊的中心节点。

这与上一节的 Node Identity 问题直接相关：如果某些长期存在的行为属性绑定在 Node 上，就必须决定 Split、Merge 后由谁继承。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



## **四、问题 3：Node 的属性到底应该由谁决定？**

这部分有一条非常直接的线索。

在 #5209 中，Omar 不只是提出要拆分 Central Node，还进一步解释：

他倾向于让停靠进去的 Window 向节点提供或更新其行为属性，而不是把长期持久化的行为状态全部保存在 Node 内。

他举的例子就是 `View` 窗口可以决定其所在节点采用什么 Resize Behavior。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



### **为什么这可能更合理？**

考虑一个编辑器有两种窗口：

- Scene View：允许弹性扩展，占据剩余空间。
- Inspector：偏好固定宽度，尽可能避免被拉伸。

如果这些属性属于 Dock Node，那么当用户把 Scene View 拖进 Inspector 区域时，可能出现语义错配：

原本给 Inspector 使用的节点，现在承载 Scene View，却仍然保留旧的尺寸策略。

如果属性来自 Window，则至少可以避免这类直接的错配。

不过，新的模型也会产生必须解决的问题：

当一个 Node 中同时存在两个或更多窗口，而它们提出不同约束时，谁拥有决定权？

例如 Scene View 要求最小宽度 600，Inspector 要求最大宽度 400，两者作为 Tabs 停靠到同一个节点时，该怎么办？

这属于我从作者方向推导出的设计难题。**目前没有看到 Omar 公开约束冲突的最终解决规则。**

因此不能将“Window 决定 Node 行为”理解为一套已经设计完成的 API。

## **五、问题 4：Docked Window 与普通 Window 行为不一致**

这部分在 2026 年有一个相当具体的案例。

[#9123 · Single visible window in dock hierarchy](https://github.com/ocornut/imgui/issues/9123?utm_source=chatgpt.com)

### **复现条件**

1. 将两个窗口 Dock 在一起。
2. 让它们处于一个具有 Split 层级的布局中。
3. 关闭其中一个窗口。
4. 观察仅剩的窗口。

实际表现是：仅剩的窗口仍然显示 Dock Node Tab Bar，而不是像独立窗口那样显示普通标题栏。

Omar 在 **2026 年 1 月 8 日** 的回复中明确说：

- 这是已知问题。
- 如果没有 Split 层级，情况会不同。
- 技术上不应该简单把窗口从 Dock Node 移除，因为原来的另一个窗口可能重新出现。
- 理想状态是视觉上表现为独立窗口，同时保留必要的停靠关系。
- 现有实现要解决这一点，容易增加复杂性；重写系统时应该更好地处理它。

  [image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

  GitHub



这个案例揭示了很重要的区别：

**Window 的逻辑归属与它的视觉表现，不一定需要完全绑定。**

一个窗口可以继续属于 Dock Node，以便其他窗口重新出现时恢复停靠关系；但在只有它可见时，标题栏和交互可以表现得与普通窗口一致。

这正对应 Omar 在 v3 方向中提出的目标：让 Dock Node 中只有一个窗口时，尽可能与没有 Docking 的单个窗口表现一致。

## **六、问题 5：DockBuilder 过度暴露了易变的布局树细节**

Omar 一直不愿意把 `DockBuilderXXX` 正式放入公共 API。

问题不只是函数命名不够友好。

当前 Builder 的典型调用是：

```
ImGui::DockBuilderAddNode(...);
ImGui::DockBuilderSplitNode(...);
ImGui::DockBuilderDockWindow(...);
ImGui::DockBuilderFinish(...);
```

它要求调用者操纵具体的 Node，并负责把窗口绑定到这些 Node ID。

对于初始布局，这通常有效。但用户修改布局后，应用就会面临：

- 重新打开的窗口应该进入哪个节点？
- 原节点被分割后应该选哪个子节点？
- 重启应用后如何尊重用户已经保存的布局？
- 程序化修改布局时如何不破坏用户已有的结构？

这些问题实际出现在 #5161、#5925、#6717 等讨论中。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub+2



Omar 希望重新设计 Builder，使其适合长期公开维护。

但有一个重要区别：**没有证据表明他已经决定把 Builder 改成声明式布局 DSL、CSS Flexbox 或某种特定的树形 API。**

在 #5209 中，确实有用户提出类似 Flexbox 的层次式 Builder，但这是社区建议，不能算作 Omar 已接受的 v3 方案。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



## **七、问题 6：Dock Node 尺寸约束并不完整**

[#2849 · Setting max/min size of windows while docking?](https://github.com/ocornut/imgui/issues/2849?utm_source=chatgpt.com)

这个 Issue 很早就展示了一个问题：

用户设置了窗口最大、最小尺寸约束，但停靠后仍然能够将窗口压缩到低于期望的尺寸。

原因涉及普通 Window 尺寸约束与 Docking 布局求解之间的区别。

这个 Issue 说明了约束能力的实际需求，而 Omar 在 v3 计划中也明确提出要让节点约束更容易实现。不过，并不能仅凭这两份资料断言 #2849 就是重写的直接起因。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub+1



从架构角度，这里真正复杂的不是单个 Pane 的 `min_width`，而是组合约束：

- Split 后，父节点最小尺寸应如何从子节点推导？
- Tabs 内多个 Window 的约束如何合并？
- 当前显示器尺寸无法满足全部最小约束时，采用什么退化策略？
- 用户拖动分割线时，哪些节点优先保持尺寸？
- 拖入新 Window 后，是拒绝 Docking，还是重新分配整个布局？

这些是实现一个完整约束系统必须回答的问题，但尚不能视为官方 v3 的已确定设计。

## **八、Multi-Viewport 也计划重写，但它是另一项工程**

Omar 在同一篇长文的 6.6 节还专门提出了 **Multi-Viewport v2**。

他认为 Multi-Viewport 要在各种 OS、Window Manager 和驱动下完全正确，甚至比 Docking 更难。

他公开提到的方向包括：

| 方向                     | 计划解决的问题                          |
| ---------------------- | -------------------------------- |
| 重新审视 `ImGuiPlatformIO` | 稳定平台窗口 API                       |
| 适应操作系统的实际行为            | 不能假设窗口管理器一定执行了请求的坐标或尺寸           |
| Wayland                | 现有全局窗口坐标模型与 Wayland 的权限和协议设计存在冲突 |
| No Main Viewport Mode  | 允许应用不创建传统主窗口，由 ImGui 后端管理所有窗口    |
| 平台差异                   | 改善 macOS、Linux 和其他窗口管理器兼容性       |

特别值得注意的是，他认为 Multi-Viewport 的核心代码量相对较小，平台 Backend 也可能大量复用，因此虽然平台兼容更难，代码重写本身未必比 Docking 更困难。

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



这也提醒我们，不应该把 Docking v3 和 Multi-Viewport v2 理解成同一次重构。

## **九、截至 2026 年 11 月，v3 有明确实现方案了吗？**

从目前能够查到的公开资料来看：

| 事项                  | 当前可确认情况      |
| ------------------- | ------------ |
| 重写意愿                | 明确，作者多次确认    |
| 要解决的架构问题            | 已公开一批具体方向    |
| 新的 Node Identity 设计 | 尚未公布确定方案     |
| Window 驱动 Node 行为   | 作者明确提出的候选方向  |
| Builder API 的新形式    | 尚未确定         |
| 约束求解算法              | 未找到公开设计      |
| 完整 v3 实现与迁移文档       | 未找到公开版本      |
| 自动化测试               | 正持续积累        |
| 正式合入 master         | 仍被 v3 重写计划阻挡 |

2026 年 1 月更新的 Docking Wiki 仍然写着希望第三次重写。我也没有在这次检索到的 2026 年公开 PR 中发现明确标识为 Docking v3 完整重写的提交。**因此现在能研究的是问题与设计线索，不是已经定型的新架构。**&#x20;

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub+1



## **十、对 Dockspace 最有参考价值的教训**

结合你现在做的无头 Docking / MultiView 库，我觉得最值得警惕的不是 ImGui 有多少未关闭的 Issue，而是这三个更根本的问题。

**1. Node Identity 不等于用户意图。**

你可以让节点拥有稳定 ID，但长期的布局语义必须额外定义。例如“打开属性面板时应该放到哪里”，不应永久依赖某个固定的节点编号。

**2. 节点属性到底属于结构、内容，还是某种策略？**

像最小尺寸、是否允许 Split、是否占据剩余空间这些规则，需要明确所有权，以及 Split、Merge、跨窗口迁移时的继承规则。

不要把“中央节点”作为解决所有特殊行为的通用出口。

**3. 临时不可见，不等于逻辑上不存在。**

隐藏窗口、关闭后可恢复、节点没有可见 Pane、Float 与 Docked 之间切换，都涉及这个区别。

仅剩一个窗口时，视觉上可以像独立窗口，语义上却仍然与已有 Docking Group 关联。这也是测试设计需要覆盖的行为。

如果按**对 Dockspace 长期架构的参考价值**排序，我会优先研究：

01

[Node Identity 与长期语义定位](https://github.com/ocornut/imgui/issues/5161?utm_source=chatgpt.com)

最值得深入理解。Omar 的回复非常具体，而且涉及布局模型是否能够长期维持语义一致性。

02

[Central Node 和 Window 驱动约束](https://github.com/ocornut/imgui/issues/5209?utm_source=chatgpt.com)

最接近 v3 未来设计思路的具体讨论。

03

[逻辑停靠状态与视觉窗口行为不一致](https://github.com/ocornut/imgui/issues/9123?utm_source=chatgpt.com)

2026 年作者明确表示希望通过重写更好地解决的实际案例。

**我的整体判断是：Omar 想解决的不是 Split Tree 算法本身，而是当前系统把“结构节点、业务窗口、持久化身份、行为约束和视觉表现”耦合得太紧。**

这是一种基于他公开发言和相关 Issue 的架构归纳，而不是他公布的正式设计文档。

也正因为如此，即使已经有成熟的 ImGui Test Engine，随机交互和回归测试依然无法从根本上降低某些修改的复杂性：一条行为规则的改变，可能同时影响布局树、窗口生命周期、持久化和可视状态。

对于你之前提到的 Dockspace 随机测试不断发现问题，我认为这提供了一个很实用的判断标准：**如果许多失败都集中在节点身份、属性继承、隐藏恢复和窗口状态的边界上，那么应该优先审视这些概念的职责，而不仅仅是继续扩大随机测试规模。** 有必要可以存一下 之后研究，我允许我们继续无畏重构 break change，发布前最完美的情况下可以看看我们是否还要继续无畏重构
