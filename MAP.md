# 目录结构

按文件夹写职责，一行一个，不列文件、不标行数。要改什么，先在这里找到该去的那一层。

```text
src/
  ansi.rs      剥掉终端转义序列，取纯文本
  cli.rs       命令行参数解析
  git.rs       工作区 git 状态快照
  lib.rs       模块图与分层规则（编译期测试）
  main.rs      进程入口：装配置、起后端、拉前端
  i18n/        编译进二进制的多语言词表，按 key 查
  ipc/         前后端唯一的线路
    contract/  线上传的信封形状（三元组、错误、文本）
    transport/ 管道与套接字，分平台
  server/      后端，不认任何前端
    agent/     模型循环：loop 驱动本轮、request 与服务商对话、tools 工具层
    api/       所有门口：call 读、post 写、subscribe 订阅推送
    config/    配置加载、校验、生效
    platform/  OS 原语：进程、shell、时钟、日志
    storage/   SQLite 持久化：读写、删改
  ui/          前端，只认 ipc 与会话
    lang/      前端语言包：Lang + 内联双语 + 错误翻译
    session/   各 UI 共用的会话中转层（上游 IPC、下游前端、结构缓存）
    tui/       终端前端
      app/     App 本体：回信路由、生命周期
      text/    文本换行、宽度、卡片
      theme/   配色与强调色
      zone/    分区：contract 词表、main 聊天区、resume 续选、tree 结构树
  web/         浏览器工具：browser CDP 驱动、html 转换、ops 操作、search 搜索
```
