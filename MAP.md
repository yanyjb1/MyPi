# 目录结构

按文件夹写职责，一行一个，不列文件、不标行数。要改什么，先在这里找到该去的那一层。

```text
adapters/ 接入的各种奇怪的前端。以及一些其它胶水脚本所在。
  telegram/   桥接的前端：把会话带到 telegram，只认桥的门
  pigeon/     本机两个 Agent 之间的信箱，命令行投递与取信，取完即删
src/
  ansi.rs      剥掉终端转义序列，取纯文本
  cli.rs       命令行参数解析
  git.rs       工作区 git 状态快照
  lib.rs       模块图与分层规则（编译期测试）
  main.rs      进程入口：装配置、起后端、拉前端
  i18n/        编译进二进制的多语言词表，按 key 查
  platform/    OS 原语：进程树、shell、时钟、日志、目录。垫在所有区域之下，谁都可以找它
  ipc/         前后端唯一的线路
    contract/  线上传的信封形状（三元组、错误、文本）
    transport/ 管道与套接字，分平台
  server/      后端，不认任何前端
    agent/     模型循环：loop 驱动本轮、request 与服务商对话、tools 工具层
    api/       所有门口：call 读、post 写、subscribe 订阅推送
    config/    配置加载、校验、生效
    storage/   SQLite 持久化：读写、删改
  ui/          前端，只认 ipc 与会话
    lang/      前端语言包：Lang + 内联双语 + 错误翻译
    session/   各 UI 共用的会话中转层（上游 IPC、下游前端、结构缓存）
    tui/       终端前端
      app/     App 本体：回信路由、生命周期
      text/    文本换行、宽度、卡片
      theme/   配色与强调色
      zone/    分区：contract 词表、main 聊天区、resume 续选、tree 结构树
    bridge/  桥接，方便不同的adapter接入的地方。
      adapter/  接入者相关，其下的web目录即为本项目WebUI所在。
      http/  HTTP&WebSocket服务
      manager/  多会话管理
  browser/     浏览器本体：CDP 驱动、进程与标签调度、html 转换、ops 操作、search 搜索
```

# 工具指南

splice有授权修改机制，必须先read/grep才有修改权，并发修改中，同一批内只要有一个点没有成功，就会失去授权，此时必须重新grep/read，建议使用grep（限制范围即可），结合splice的报错提示，迅速定位相关上下文，此时grep由于命中稀少，基本会全部展开。
需要一个函数内部的全量行号时，可使用read的item模式，但是更建议一次性根据给出的关键行，把关键行内部的代码完整替换。
write/splice写入后，只要有成功写入，就会被fmt，因此，不满意/不确定需要再次编辑时，请务必重新read/grep。

# 特别行为

如果使用的网关质量较差，用户会持续地发1来保证连接正常，除此之外无实意。

# 外部审查

在对局部代码进行优化时用户会将已有目标代码交给另一个AI（氢基米）审核，并由其指控，此时，你的主要任务是：核验相关代码，结合已知的设计原则等，核验指控，区分有价值的和无价值的，并对有价值的指控进行针对性修复。

这是本项目特有的一种工作模式。

# pigeon协作

通过adapters/pigeon/app.py的脚本与agy通信，获取来自agy的方向指导。
agy整体而言是一个知识库比较落后、但是经验非常丰富的聪明模型，用户会与它完成具体的决策（它说人话），但最后的machen信号仍然由用户给出。
具体什么时候写信，取信，由用户通知。

用法：
投信：python adapters/pigeon/app.py post --from mypi --to agy --body <text>
取信：python adapters/pigeon/app.py fetch mypi

取信时不允许使用head参数，信件看一次就没，不允许head，必须完整阅读。

这是本项目特有的一种工作模式。