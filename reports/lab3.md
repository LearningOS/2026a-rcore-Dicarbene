# Lab3：进程管理与 Stride 调度

## 一、实现功能

实现 `spawn`，从内置 ELF 创建子进程并返回 PID，父进程可以等待并获取退出码。实现 Stride 调度和优先级接口，优先运行累计 stride 最小的就绪进程。迁移时间查询和匿名映射接口，使其通过当前进程的页表访问用户内存。

个人实验用时约 10 小时，为本人回顾估算。

## 二、实现思路

### 1. 从 ELF 创建进程

spawn 先从进程地址空间读取以零结尾的应用名。逐字节检查映射与读权限，因此名称可以跨页。读取上限取自内置应用名的最大长度，遇到非法地址、缺少终止符或不存在的应用时返回 -1。

找到 ELF 后复用进程构造函数。它为子进程建立地址空间、TrapContext、PID 和内核栈。子进程用弱引用指向父进程，父进程在子进程列表中持有强引用。新进程进入就绪队列后从 ELF 的入口执行，退出后由 waitpid 回收。

### 2. Stride 调度与优先级

每个进程初始优先级为 16，stride 为 0。调度器扫描就绪队列，取 stride 最小的一项，遇到相同值时按入队顺序选择。被选进程的 stride 加上 `2^64 / priority`。优先级越高，每次增长越少，因此持续就绪的进程会被更频繁地选中。

累加值用 u128 保存。分子足够大，即使优先级是 isize::MAX，步长也至少为 2。set_priority 拒绝小于 2 的值，成功时返回新优先级。变更只影响以后的步长，已经累计的 stride 保留。

### 3. 迁移地址空间接口

ch5 通过当前进程取得 MemorySet。get_time 验证完整目标范围后按页复制，避免后一页无效时改写前一页。mmap 和 munmap 复用 lab2 的区间检查和物理页管理。fork 按现有 MemorySet 的区域复制映射，exec 更换地址空间时同步更新堆起点和 program break。

## 三、问答题

### 1. 8 位 stride 回绕后，直接比较会选中谁？

p2 从 250 加 10 得到 260。用 8 位无符号数保存后变成 4，直接比较 4 和 255 会再次选择 p2。按真实的累计值 255 和 260 比较，应该先选择 p1。

### 2. 最大值和最小值的差为何至多为 BigStride / 2？

对从零开始并持续参与调度的进程集合，用归纳法。初始差为 0。每轮只给最小值加步长，步长不超过 BigStride / 2。更新后，新最大值至多为旧最小值加 BigStride / 2，新最小值也不会比旧最小值更小，所以这个间距上界仍然成立。若允许新进程随时以零加入或阻塞很久后恢复，则需要单独处理其入队时的 stride，才能继续使用这个间距不变量。

### 3. 如何让 BinaryHeap 弹出实际 stride 最小的项？

令半周期为 2^63。若真实差距严格小于半周期，`other - self` 的环绕差为正且小于半周期时，self 在实际时间线上较小，比较器应返回 Greater，使最大堆优先弹出它。

```rust
fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
    const HALF: u64 = 1 << 63;
    match other.0.wrapping_sub(self.0) {
        0 => Some(Ordering::Equal),
        HALF => None,
        distance if distance < HALF => Some(Ordering::Greater),
        _ => Some(Ordering::Less),
    }
}
```

恰好相隔半周期时，仅凭两个环绕后的值无法判定先后。真正用于 BinaryHeap 的 Ord 需要配套的间距约束和一致的相等定义。本实验使用更宽的累计值和就绪队列扫描，实际调度不依赖这个环形比较器。

## 四、验证

在 WSL Ubuntu 中用课程固定的 Rust 工具链、QEMU 8.2.2 和同版本官方检查器运行 `make -C ci-user test CHAPTER=5`。原版检查通过 **15/15**，lab1–lab3 报告检查均通过，命令退出码为 0。完整日志见 `tmp/lab3-official.log`。

官方检查器的分数没有单列 Stride 的比例。用同次日志中优先级 5 到 10 的六个进程，分别用运行计数除以各自优先级，归一化计数的最大值与最小值之比为 **1.045**，低于题目要求的 1.5。

另在本地临时加入 `tmp/ch5_spawn_paths.rs`，验证不存在的应用、无效指针、文件名跨到未映射页时会返回 -1，以及跨两个物理页的有效名称能创建子进程并获取退出码。测试输出 `SPAWN PATH CHECKS COMPLETE`，完整日志见 `tmp/lab3-path-check.log`。

## 五、荣誉准则与辅助来源说明

1. 已确认的交流对象为 OpenAI Codex。11 月 25 日在「查看 OpenCamp 下一阶段任务」中请 Codex 辅助迁移内存接口、实现 `spawn` 和 Stride 调度、检查跨页文件名和无效地址，并根据六个进程的输出核对调度比例，随后根据实现和日志起草报告。本人确认未就本实验与助教、同学或其他 AI 交流。
2. 参考资料包括 [第四章实验报告与实现](lab2.md)、[第五章练习](https://learningos.cn/rCore-Tutorial-Guide/chapter5/4exercise.html)、课程模板与固定版本的官方测试。未使用其他学员的解答。
3. 本次代码与文档使用了上述 AI 辅助。提交者应核实并理解实现，如实说明自己完成的工作。已知辅助可能影响课程评分。
4. 本次代码用于实现和验证实验接口。提交者应按课程要求妥善保管实验代码，遵守 [课程荣誉准则](https://github.com/LearningOS/rCore-Tutorial-Guide/blob/main/source/honorcode.rst)。
