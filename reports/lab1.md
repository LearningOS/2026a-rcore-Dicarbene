# Lab1：多道程序与分时多任务

## 一、实现功能（200 字以内）

实现系统调用 `sys_trace`：请求 0 读取指定地址的一个字节，请求 1 写入 `data` 的低八位，请求 2 查询当前任务的系统调用次数，其他请求返回 -1。各任务的计数保存在任务控制块中，在系统调用分发前递增，因此查询包含本次调用，任务切换不会混淆统计。对超出计数数组范围的查询返回 -1。本章按实验要求直接访问有效用户地址，不实现地址空间隔离。

个人实验用时约 10 小时，为本人回顾估算。

## 二、实现思路

### 1. 按任务保存统计

在 `os/src/task/task.rs` 的 `TaskControlBlock` 中增加 `syscall_times`，每个任务保存 500 个 `usize` 计数器，覆盖本章包括 410 在内的系统调用号。任务管理器初始化时将数组清零，后续调度只切换当前任务，不清空计数。

在 `os/src/task/mod.rs` 中增加记录和查询接口，通过 `current_task` 找到正在运行的任务。数组访问使用 `get_mut` 和 `get`，避免把用户提供的编号直接用于数组索引。未调用过的范围内编号返回 0，范围外查询返回 -1。

### 2. 在系统调用入口计数

在 `os/src/syscall/mod.rs` 的 `syscall` 入口处记录调用，再执行分发。这样 `sys_trace` 查询自身次数时能看到当前调用，`sys_yield` 的计数也会记在让出 CPU 的任务上。若在分发之后记录，查询会少算一次，`sys_exit` 这类不返回的调用还会漏记。

记录函数在返回时释放 `UPSafeCell` 的内部借用，避免跨越任务切换保留 `RefMut`，从而与调度器再次访问任务管理器发生冲突。

### 3. 字节读写

在 `os/src/syscall/process.rs` 中按请求编号处理。读写使用 `u8` 指针和 volatile 操作；读取结果为 0～255，写入时转换为 `u8`，只保留低八位。volatile 操作用于显式执行该次内存访问，并不提供地址合法性检查或内存隔离。

本章尚未启用分页，不要求实现无效地址检查；直接解引用错误地址仍可能导致内核异常。后续地址空间实验需要重新处理地址转换与访问权限。

## 三、简答题

### 1. 用户态执行非法操作时会发生什么？

使用仓库提供的 **RustSBI 0.4.1** 固件，版本来源为 `bootloader/rustsbi-qemu.toml`。三个测试的操作和异常处理路径如下：

| 测试 | 操作 | 异常及内核处理 |
| --- | --- | --- |
| `ch2b_bad_address` | 向地址 0 写入一个字节 | 写访问异常，内核输出 `PageFault in application` 并终止该任务 |
| `ch2b_bad_instructions` | 在 U 态执行 `sret` | 非法指令异常，内核输出 `IllegalInstruction in application` 并终止该任务 |
| `ch2b_bad_register` | 在 U 态读取 `sstatus` | 非法指令异常，内核输出同类提示并终止该任务 |

本章 Trap 处理器将 `StoreFault` 和 `StorePageFault` 都打印为 `PageFault in application`，因此日志中的文字不代表已经启用分页。`sret` 和读取监督态 CSR 都不能由 U 态直接执行。异常任务被标记为 `Exited` 后，调度器继续运行其他就绪任务，而不是让整个系统随用户程序一起崩溃。

实际运行中依次出现一条地址 0 的写访问异常和两条非法指令异常，随后正常运行 Hello World、计算、休眠及 trace 测试，与上表一致。

### 2. `__alltraps` 和 `__restore` 的作用

`__alltraps` 是 Trap 入口：交换用户栈与内核栈，保存用户寄存器、`sstatus` 和 `sepc`，随后调用 Rust 的 `trap_handler`。`__restore` 根据内核栈中的 `TrapContext` 恢复上下文，最终通过 `sret` 返回用户态。下面按本仓库 `os/src/trap/trap.S` 的实际代码回答，不依赖教材行号。

#### （1）刚进入 `__restore` 时，`sp` 是什么？有哪两种场景？

`sp` 指向当前任务内核栈上的 `TrapContext`，也就是保存通用寄存器、`sstatus` 和 `sepc` 的上下文起始地址。

两种使用场景：

1. 首次运行应用时，内核预先构造 `TrapContext`，经任务上下文切换进入 `__restore`，以应用入口地址作为 `sepc`，进入 U 态。
2. 用户程序发生系统调用、中断或异常，进入 `__alltraps` 并完成处理后，执行 `__restore`，恢复用户程序的执行现场。如果中途发生调度，仍会在该任务重新获调度后完成返回。

#### （2）为什么单独恢复 `sstatus`、`sepc` 和 `sscratch`？

- `sstatus` 保存返回时使用的特权状态。其中 `SPP` 决定 `sret` 返回到哪个特权级；应用初始化时将其设为 User。`SPIE` 等字段还参与中断使能状态恢复。
- `sepc` 保存返回后的指令地址。系统调用处理会将它加 4，跳过已处理的 `ecall`；异步中断则需要返回被打断的位置。
- `sscratch` 暂存从上下文 `x[2]` 取出的用户栈指针，供最后一次栈交换使用。返回用户态后，它保存内核栈顶，供下次 Trap 入口切换栈。

`t0`、`t1`、`t2` 在这里充当临时寄存器，稍后才恢复它们原来的用户态值。

#### （3）恢复通用寄存器时为什么跳过 `x2` 和 `x4`？

`x2` 是 `sp`，此时还用于定位内核栈上的上下文，不能提前改成用户栈指针，因此最后通过 `sscratch` 交换恢复。

`x4` 是 `tp`。本章应用未使用线程局部存储，汇编明确省略了它的保存和恢复。这是本章框架的简化；后续若依赖 `tp`，就需要保存和恢复其值。

#### （4）`__restore` 中最后一次交换后，`sp` 和 `sscratch` 各是什么？

执行 `csrrw sp, sscratch, sp` 前，内核已经释放当前 `TrapContext` 占用的栈空间。交换后，`sp` 为用户栈指针，`sscratch` 为当前任务的内核栈顶，供下一次 Trap 使用。交换栈本身不会改变特权级。

#### （5）哪条指令完成返回用户态？为什么？

`sret` 完成特权级切换，并将执行位置设为 `sepc`。因为保存的 `sstatus.SPP` 为 User，所以它返回 U 态；`sret` 本身并不保证一定返回 U 态，返回级别取决于 `SPP`。

#### （6）`__alltraps` 入口处交换后，`sp` 和 `sscratch` 各是什么？

在入口执行 `csrrw sp, sscratch, sp` 后，`sp` 指向当前任务内核栈顶，`sscratch` 保存刚被打断的用户栈指针。随后在内核栈上分配 `TrapContext`，并把用户栈指针存入 `x[2]`。

#### （7）从 U 态进入 S 态发生在哪条指令？

对系统调用而言，是 U 态的 `ecall` 触发异常，处理器按 Trap 机制进入 S 态，并跳转到 `stvec` 指定的入口。进入 `__alltraps` 时已经处于 S 态，栈交换指令不负责提权。

更一般地，时钟中断或其他异常也可以触发 U→S 切换，不一定对应一条主动发起系统调用的指令；Trap 是否交由 S 态处理还取决于异常与中断委托配置。

## 四、实验环境与验证

- 实验分支：`ch3`。
- 系统：WSL Ubuntu 24.04。
- Rust：`nightly-2024-05-02`。
- 目标：`riscv64gc-unknown-none-elf`。
- `cargo-binutils`：0.4.0。
- QEMU：8.2.2。
- 固件：仓库自带 RustSBI 0.4.1。
- 官方检查器版本：`7d61ec55b58eed6ca7052917846c1b87af34563a`。
- 官方用户测试版本：`a0593662ad55d670ba8c27ce1763347cd0dd552f`。

2026-10-31 在仓库根目录执行官方测试。由于本机 Git 将 GitHub HTTPS 地址改写成 SSH，第一次运行在下载 `riscv` 依赖时认证失败，尚未启动内核。随后只为测试进程覆盖地址改写，并让 Cargo 使用 Git CLI，未修改全局 Git 配置：

```sh
GIT_CONFIG_COUNT=1 \
GIT_CONFIG_KEY_0=url.https://github.com/rcore-os/.insteadOf \
GIT_CONFIG_VALUE_0=https://github.com/rcore-os/ \
CARGO_NET_GIT_FETCH_WITH_CLI=true \
make -C ci-user test CHAPTER=3
```

重跑结果：官方检查器 **7/7 通过**，输出 `Report for lab1 found.`，完整命令退出码为 **0**。检查器会给成功标记附加随机数字，本次汇总为 `Test passed2794: 7/7`。完整构建与测试日志保存在本地 `tmp/lab1-check.log`（该目录被 Git 忽略，不随作业提交）。

验证内容包括休眠与时间接口，以及 trace 测试对调用次数、自身计数、字节读写和代码区读取的断言。基础测试同时运行了三个非法操作测试，内核终止对应任务后继续运行其他应用。所有任务结束时打印 `All applications completed!`，这是本章框架的结束路径，本次检查器正常接受该结果。

此外，`cargo fmt --manifest-path os/Cargo.toml -- --check` 与 `git diff --check` 均通过。以上为本地测试结果，尚未推送到 GitHub，未核实 Actions 与 OpenCamp 的线上成绩。

## 五、荣誉准则与辅助来源说明

1. 已确认的交流对象为 OpenAI Codex。10 月 14 日在「查看 OpenCamp 下一阶段任务」中核对了课程安排、各章任务和提交要求。10 月 31 日在「检查项目任务内容」中核对课程仓库和评测要求，并请 Codex 辅助实现 `sys_trace` 和按任务计数、解释 Trap 汇编、配置 QEMU、排查依赖下载问题、运行官方测试和起草报告。11 月 25 日在前一段对话中用独立副本复跑官方检查，结果为 7/7。本人确认未就本实验与助教、同学或其他 AI 交流。
2. 参考资料为本仓库的课程说明、内核源码、固件版本记录，以及 LearningOS 的[第三章练习](https://learningos.github.io/rCore-Tutorial-Guide/chapter3/5exercise.html)、[荣誉准则](https://learningos.github.io/rCore-Tutorial-Guide/honorcode.html)和官方测试 `ch3_trace.rs`。实现依据接口规范编写，未复制其他学员的解答代码。
3. 本次代码与文档使用了上述 AI 辅助，不声明它们由提交者未经辅助独立完成。提交者应核实内容并理解实现；辅助信息可能降低实验难度并影响课程评分。
4. 本次修改不包含破坏或妨碍系统正常运转的功能。提交者仍需遵守课程关于保管实验代码、不向他人复制或公开解答的要求。此处如实披露辅助方式，不代替提交者作未经核实的个人承诺。
