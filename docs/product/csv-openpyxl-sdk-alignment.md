# CSV SDK 与 openpyxl 对齐结果

日期：2026-09-16。对照 openpyxl 3.1.5；这里区分「能使用对象」与「前端能还原效果」，不把接口存在等同于完整显示支持。

## 使用方式

```python
from openpyxl.styles import Font, PatternFill, Border, Side, Alignment

wb = await locus.csv.load_workbook("Locus/knowledge/design/items.csv")
ws = wb.active
ws["A1"].font = Font(name="Arial", size=12, bold=True)
ws["A1"].fill = PatternFill("solid", fgColor="E2F0D9")
ws["A1"].alignment = Alignment(horizontal="center", wrap_text=True)
ws["A1"].border = Border(bottom=Side(style="thin", color="808080"))
ws["B2"].number_format = "0.0%"
await wb.save()
```

使用真实的 openpyxl Workbook / Worksheet / Cell / 样式对象，仅在 CSV 所需行为上扩展：异步加载保存、源码坐标、保留文本、保留合并区域的被覆盖值。支持 `cell`、A1 范围、`iter_rows` / `iter_cols`、`append`、复制样式和 `NamedStyle`。旧 `read_view` / `patch_view` 保留。

Python 工具描述和 SDK 文档已直接说明 openpyxl 用法。托管 Python 的打包流程安装 openpyxl 3.1.5；外部 Python 通过 `python/requirements-csv.txt` 安装。其他 SDK 功能保持标准库依赖。

## 前端样式覆盖与差距

| openpyxl 对象 / 能力 | 已实现 | 近似显示或仍缺失 |
| --- | --- | --- |
| `Font` | 字体、含小数的磅字号、粗体、斜体、文字颜色、删除线、单/双下划线、上下标 | 会计下划线映射为普通单/双下划线；上下标、字形度量使用浏览器排版。未支持 outline / shadow / condense / extend；family / charset / scheme 不作为 Excel 字体目录保留 |
| `PatternFill` | 纯色与全部图案名称 | 图案使用 CSS 纹理近似，线距、密度与方向不保证和 Excel 像素一致 |
| `GradientFill` | 无 | 线性与路径渐变、渐变停靠点仍未支持，保存前明确报错 |
| `Border` / `Side` | 上右下左独立边框；thin / medium / thick / hair / dashed / dotted / double | dashDot、dashDotDot 及其 medium / slant 变体映射为虚线；diagonal、inside horizontal / vertical、start / end 未支持。无装饰边框时恢复 Locus 网格线 |
| `Alignment` | 常规水平/垂直对齐、换行、缩进、文字方向、旋转、竖排、shrink-to-fit | distributed / justify 使用浏览器近似；缩放、旋转和上下标组合不保证 Excel 排版一致。fill / centerContinuous / relativeIndent / justifyLastLine 未支持 |
| `number_format` | 使用 SheetJS SSF：小数、千分位、百分比、货币、科学计数、分数、常见日期时间及格式分段 | `[Red]` 等格式分段颜色未应用到文字；Excel 完整区域、日历和语言规则未还原。无法解析的格式回退原文，不修改 CSV 值 |
| `Color` | RGB / aRGB、indexed、标准 Office theme + tint | indexed / theme 保存时转成 RGB，未实现可切换的 Excel 主题。aRGB 的 alpha 按 Excel 样式语义忽略；旧 Locus 语义色继续保留 |
| `NamedStyle` / 样式复制 | 使用真实对象，支持命名样式赋值和 `copy.copy` | 保存实际格式，不保存可重新编辑的 Excel 命名样式目录 |
| 合并与尺寸 | 合并/取消合并、行高、列宽、隐藏行列、冻结前导列 | 列宽按固定字符度量换算，字体变化后不会重算 Excel 列宽。冻结行仍缺失；隐藏合并区域的行时，仍显示可见区域及源锚点值 |
| 条件格式 | 单个字面量比较的 `CellIsRule`，含数值相等/不等；支持重新加载、修改、删除；保留已有 Locus 动态规则 | 字体条件覆盖限字体名、字号、粗体和颜色；其他字体效果仍限直接样式。FormulaRule、between/notBetween、色阶、数据条、图标集、stopIfTrue 和完整 Excel 优先级语义未支持 |

常用的单元格排版已覆盖。若目标是完整还原 openpyxl 可描述的 Excel 样式，剩余工作主要有六组：渐变、复杂条件格式、特殊边框、特殊字体效果、Excel 专用排版、数字格式的颜色与区域规则。它们复杂度差异很大，因此不使用一个没有统一分母的完成百分比。

条件格式的 `fill` / `border` / `alignment` 当前作为完整组件覆盖；更细的差异样式属性继承也仍需扩展。Locus 默认字体与颜色继续采用桌面工具的主题，不强行替换为 Excel 的默认外观。

## CSV 简化边界

- 一个 CSV 只有一张工作表；不加入工作簿密码、工作表/单元格保护、打印、分页、图表、图像、数据验证或公式计算。这些不计入 CSV 样式显示缺口。
- 所有读取值保留为字符串，避免 `001`、日期和公式文本被自动转换；不沿用 Excel 的单元格 32,767 字符限制。数值、布尔、日期和空值赋值可转换成 CSV 文本。
- 仅改样式不会重写 CSV。值变化只替换对应字段，保留其他字段的引号、BOM、分隔符和各条记录的换行；合并不删除覆盖值。
- 版本检查与批量保存封装在 SDK 内，不要求 Agent 操作版本号或文件锁。保留现有工作区授权、知识库编辑权限和并发修改检测。
- 行列插入/删除、移动范围使用表格编辑器；本适配层提供单元格值编辑和追加行。纯样式操作不会生成空白 CSV 数据。
- 尚未实现的显式 Excel 功能在保存前报错；不同于已声明的 CSS 近似渲染和数字格式原文回退。

## 持久化与兼容

`.csv.view` v4 增加 openpyxl 格式组件和逐行尺寸。v1/v2/v3 读取不写盘，首次保存新能力时显式升级；升级保留原有列、排序、过滤、稀疏规则和合并区域，禁止降级。重复样式会在保存前合并为行/范围规则。

写入前验证整批布局与 CSV 内容；版本冲突不自动重试。CSV 写入失败时尝试恢复本次写入的配套文件。这是两个文件的协调保存，不承诺进程崩溃或断电时的跨文件事务。

## 验证

- Python SDK 单元测试：79 项通过，包含真实 openpyxl 对象、规则压缩、命名样式、条件格式往返、版本冲突、长文本、追加空行和 CSV 字节保留。
- CSV / SDK 前端测试：107 项通过，包含真实 Tabulator 的交互回归。
- `bun run typecheck` 与 `bun run typecheck:test` 通过。
- Rust CSV 定向测试：20 项通过，包含旧格式迁移、内容/样式保存、无效批次不落盘和重复保存。
- 独立浏览器页面挂载实际 `CsvGrid` / 合并层进行视觉检查；确认字体、下划线、删除线、填充、边框、换行、旋转、数字格式和合并显示，并修复内部文本层未继承字体/装饰的问题。

前端复用现有 CSV 网格、合并层、尺寸测量与全局主题；没有新增页面控件、badge、chip 或私有按钮样式。

对照资料：[openpyxl 样式](https://openpyxl.readthedocs.io/en/stable/styles.html)、[条件格式](https://openpyxl.readthedocs.io/en/stable/formatting.html)、[SheetJS 数字格式](https://docs.sheetjs.com/docs/csf/features/nf/)。
