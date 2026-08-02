# Research: Flutter 上做「现代 SwiftUI 感」UI 的实践者

> 调研日期：2026-03-25  
> 问题：在 Flutter 上自研现代 iOS/SwiftUI 风格（而非 Material / 传统 Cupertino / shadcn）是否有真实实践者？成熟度如何？对 Minos mobile 有何启示？

## TL;DR

**有实践者，而且最近 12 个月明显升温**——但没有「开箱即用、生产级、跨端一致的 SwiftUI-modern 完整组件库」。

业界实际走的是三条路：

| 路线 | 代表 | 成熟度 | 是否适合 Minos |
|------|------|--------|----------------|
| A. 自研品牌设计系统（纯 Flutter widgets） | Wonderous / gskinner；大量商业 App；`mix` 作样式层 | **最高**，已被 Flutter 官方点名为 custom design system 范例 | **主路径** |
| B. 平台原生嵌入（Platform View 套 UIKit/SwiftUI） | Serverpod `cupertino_native`；`cupertino_native_better`；`adaptive_platform_ui` | PoC～早期，iOS 像素级真，Android 只能 fallback | 可作 **iOS 点缀**（tab/toolbar），不能当双端设计系统 |
| C. 纯 Flutter 仿 Liquid Glass / iOS 26 | `liquid_glass_renderer`、`liquid_glass_widgets` | 实验性，效果强、性能与生产风险高 | 可作 **视觉效果层**，不宜整库绑定 |

官方方向也在对齐 A：Flutter 正在把 Material/Cupertino **从 SDK 核心拆成独立 package**，明确承认「大量团队在做 bespoke design system，却在和 Material 纠缠」。

## 1. 问题背景（社区共识）

- 内置 `cupertino` ≈ 旧 UIKit 复刻，**跟不上** iOS 17–26 / SwiftUI 现代语言（大圆角、floating tab、liquid glass、更疏的层级）。
- 社区长期吐槽 Flutter 落在「uncanny valley」：很像原生，但差一口气。
- Serverpod 创始人 Viktor Lidholt（前 Flutter 相关从业者）2025-09 发文：  
  [Is it time for Flutter to leave the uncanny valley?](https://medium.com/serverpod/is-it-time-for-flutter-to-leave-the-uncanny-valley-b7f2cdb834ae)

## 2. 路线 A — 自研设计系统（主实践）

### 2.1 Flutter 官方自己点名

[Flutter for SwiftUI Developers](https://docs.flutter.dev/get-started/flutter-for/swiftui-devs) 明确写：Flutter 可遵循任意设计系统，并列出：

- Custom Material widgets  
- **Your own custom widgets**  
- Cupertino  
- 并指向 **Wonderous** 作为 custom design system 范例：  
  https://flutter.gskinner.com/wonderous/

### 2.2 Wonderous / gskinner（旗舰案例）

- 仓库：https://github.com/gskinnerTeam/flutter-wonderous-app（~4.5k★）
- Flutter 团队合作、上架 App Store / Play
- **完全不走「套系统控件皮肤」**，而是自绘导航、动效、排版、过渡
- 目标是证明 Flutter 能做高保真、高创意 UI，而不是复刻 HIG 控件目录
- gskinner 更早还有 Flutter Vignettes（Interact 展示），同一方法论：设计师 + 工程师在 Flutter 原语上做品牌体验

**启示**：真正好看的 Flutter 移动端，行业标杆就是这条路——**token + 自研组件 + 强动效**，不是 CupertinoApp 换皮。

### 2.3 平台/桌面侧的「完整第三方设计系统」已验证

这些证明「Flutter 上做完整非 Material 设计系统」可行且有长期维护者：

| 包 | 设计语言 | 说明 |
|----|----------|------|
| [`fluent_ui`](https://pub.dev/packages/fluent_ui) | Windows Fluent | 社区维护多年，完整 App 壳 |
| [`macos_ui`](https://pub.dev/packages/macos_ui) | macOS HIG | 完整 macOS 控件语言 |
| [`yaru`](https://pub.dev/packages/yaru) | Ubuntu/GNOME | Canonical 相关，生产使用 |
| [`forui`](https://pub.dev/packages/forui) / [`shadcn_ui`](https://pub.dev/packages/shadcn_ui) | shadcn/web | 证明「非 Material 组件库」市场存在，但气质偏 Web，**不是** iOS-modern |

**缺口**：Fluent/macOS/Yaru 都有完整库；**iOS-modern / SwiftUI-modern 反而没有对等的、生产级、跨 Android 一致的完整库**。iOS 侧要么旧 Cupertino，要么实验性 liquid glass，要么原生嵌入。

### 2.4 样式系统工具层：`mix`

- https://pub.dev/packages/mix / https://github.com/conceptadev/mix  
- 明确卖点：**不绑定 Material**，用 token / styler / variant 做 design system
- 适合作为「自研组件」的样式引擎，而不是现成 iOS 控件集

### 2.5 商业产品侧（模式，非逐一审计源码）

长期存在的模式是：消费级 Flutter App（Reflectly 一代、各类 Fintech/Health）**品牌 UI 自研**，仅在滚动物理、字体、手势上贴近平台。  
公开可复现的「完整开源 iOS-modern 业务 App」很少——因为设计系统是产品壁垒，通常不开源。  
可开源对照的主要是 Wonderous 这类 showcase。

## 3. 路线 B — 原生 Platform View 嵌入（2025 新浪潮）

### 3.1 Serverpod `cupertino_native`

- https://github.com/serverpod/cupertino_native / https://pub.dev/packages/cupertino_native  
- 作者 Viktor Lidholt：用 **Platform View 嵌真实 UIKit/SwiftUI Liquid Glass 控件**
- 组件：Slider / Switch / Segmented / Button / SF Symbol / PopupMenu / TabBar
- 自述：能跑、够快，但是周末 vibe-coded 的 PoC；愿景是过渡到「更好的纯 Flutter Cupertino」之前的桥梁
- Android：合理 Flutter fallback，**不是**同一套原生玻璃
- 需要较新 Xcode（文档提到 Xcode 26 beta）

### 3.2 后续 fork / 增强

- `cupertino_native_better`（修复 release 版本探测、SF Symbol fallback、modal z-order 等）
- `cupertino_native_plus`
- `adaptive_platform_ui`：iOS 26 native toolbar/tabbar + 旧 iOS Cupertino + **Android Material**（与 Minos「双端都不要 Material」冲突）

**关键限制（作者与 README 都承认）**：

- Platform View 不宜进长列表
- 与 Flutter 路由/sheet/键盘的 z-order 需 observer 协调
- 双端视觉不可能真正一致（iOS 真原生，Android 仿或 Material）
- 维护绑定 Apple 私有/新 API 风险高

**启示**：若 Minos 要「iPhone 上 tab bar 绝对真」，可局部用 B；若要 **iOS+Android 同一现代感**，B 不能当底座。

## 4. 路线 C — 纯 Flutter 仿 iOS 26 Liquid Glass

| 项目 | Stars 量级 | 要点 |
|------|------------|------|
| [whynotmake-it/flutter_liquid_glass](https://github.com/whynotmake-it/flutter_liquid_glass) → `liquid_glass_renderer` | ~400+ | Shader 折射/模糊；**实验性**；要 Impeller；作者警告勿盲目上生产 |
| [sdegenaar/liquid_glass_widgets](https://github.com/sdegenaar/liquid_glass_widgets) | ~400 | 完整 glass 组件库 + Apple Music/Messages/News demo；跨平台 shader 路径 |
| [renancaraujo/liquido](https://github.com/renancaraujo/liquido) 等 | 更小 | 效果/研究向 |

这些证明：**社区在用力把 SwiftUI/iOS26 美学搬进纯 Flutter**，而且已经有可运行的 Music/Messages 复刻 demo。  
但共同问题是：性能、中低端机、无障碍（Reduce Transparency）、以及 API 稳定性——更适合「导航条/浮动岛」等少量表面，不适合全 App 每个 cell 都上真玻璃。

## 5. Flutter 官方战略：为 custom design system 让路

文档：[Evolving Flutter’s Design Systems / decouple-design](https://docs.google.com/document/d/189AbzVGpxhQczTcdfJd13o_EL36t-M5jOEt1hgBIh7w/)（go/decouple-design，2025-07 更新）

核心点：

1. 把 **Material 与 Cupertino 移出 SDK**，变成 pub 上的一等 package  
2. 强化 core `widgets` 里的 **headless / 无视觉偏见原语**（文本选择、页面转场等从 Material 抽出）  
3. 明确动机：大量团队做 **bespoke design system**，却被迫和 Material 缠斗；M2→M3 迁移伤筋动骨；Apple iOS26 / M3 Expressive 又要再来一轮  
4. 官方 **不做**「统一 adaptive 万能库」或「官方新 theming 系统」——把空间留给社区与产品自研  
5. 新 iOS26 / Liquid Glass 风格的官方 Cupertino 更新，计划放在拆包完成之后

这等于官方背书：**「自研设计系统」不是旁门，而是框架演进的一等公民场景。**

## 6. 和 Minos 目标的映射

Minos 约束回顾：

- 双端（iOS + Android）**同一**现代美观  
- 拒绝 Material 视觉  
- 拒绝 shadcn/web 气质  
- 内置 Cupertino 又偏旧 UIKit  
- 已有干净的 domain/application/FRB，只该动 `lib/ui`

| 候选 | 评价 |
|------|------|
| 继续 shadcn_ui | 已验证不符合产品气质 |
| 纯 Cupertino | 旧，达不成 SwiftUI-modern |
| 整库 `cupertino_native*` | iOS 真、Android 假；Platform View 与列表/聊天冲突；过重 |
| 整库 `liquid_glass_widgets` | 效果惊艳但实验性；绑定第三方演进风险 |
| **自研 Minos DS（A）+ 可选局部 glass/native（C/B）** | 与 Wonderous/官方方向一致；双端可控；符合仓库 latest-only |

### 推荐技术姿态（研究结论，非实施承诺）

```
视觉源 of truth: Minos tokens + 自研组件（纯 Flutter）
样式层:        自研 Theme / 可选 mix
行为:          iOS 向（弹性滚动、edge back、sheet 语义）
质感增强:      少量 BackdropFilter / 自研 soft surface；
               必要时局部 liquid_glass_renderer（tab/navbar only）
不做:          Material 可视组件；shadcn；全量 Platform View
```

## 7. 风险与诚实边界

1. **没有「站在巨人肩膀上的完整 iOS-modern 库」可直接买时间**——和 Windows/macOS/Linux 生态不对称。  
2. 自研 DS 的成本在 **设计决策 + 组件完备度 + 动效**，不在 Flutter 能力本身。Wonderous 证明上限很高。  
3. 若执着「像素级等于系统 App」，只有 B（原生嵌入）在 iOS 上成立，但会牺牲 Android 一致与架构简洁。  
4. Liquid Glass 全量上生产仍被包作者自己标红警告。  
5. Flutter 拆包完成后，官方新 Cupertino 可能会补上现代 iOS；时间线以季度计，**不能**当 Minos 近期依赖。

## 8. 主要一手来源

- Flutter docs — SwiftUI 开发者指南（custom design system + Wonderous）  
  https://docs.flutter.dev/get-started/flutter-for/swiftui-devs  
- Flutter design systems decoupling proposal  
  https://docs.google.com/document/d/189AbzVGpxhQczTcdfJd13o_EL36t-M5jOEt1hgBIh7w/  
- Wonderous  
  https://github.com/gskinnerTeam/flutter-wonderous-app  
- Serverpod cupertino_native + blog  
  https://github.com/serverpod/cupertino_native  
  https://medium.com/serverpod/is-it-time-for-flutter-to-leave-the-uncanny-valley-b7f2cdb834ae  
- liquid_glass_renderer  
  https://pub.dev/packages/liquid_glass_renderer  
- liquid_glass_widgets  
  https://github.com/sdegenaar/liquid_glass_widgets  
- mix  
  https://pub.dev/packages/mix  
- fluent_ui / macos_ui / yaru / forui（对照：完整第三方 DS 已存在于非 iOS-modern 领域）

## 9. 一句话结论

> **有实践者：官方 showcase（Wonderous）、平台 DS 库（Fluent/macOS/Yaru）、样式引擎（mix）、以及 2025 起的 Liquid Glass 双路线（原生嵌入 + Shader 复刻）。**  
> **没有**：可直接采用的、生产级、双端统一的「SwiftUI-modern 完整 UI 套件」。  
> **Minos 若要双端都美且非 Material，行业验证过的路就是自研设计系统；原生嵌入和 glass shader 只适合做局部加速，不适合当底座。**
