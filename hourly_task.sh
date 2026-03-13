#!/bin/bash

# 环境变量配置（关键！cron 默认无用户环境变量）
SHELL=/bin/bash
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/home/ubuntu/.cargo/bin/cargo

# 日志配置
LOG_FILE="/home/ubuntu/hourly_task.log"
TIMESTAMP=$(date "+%Y-%m-%d %H:%M:%S")

# 任务内容示例（根据需求修改）
main() {
    cd /home/ubuntu/nori_workspace/mina_bridge/
    #make submit_devnet_state
    /home/ubuntu/.cargo/bin/cargo run --manifest-path core/Cargo.toml --release -- submit-state --devnet
}

# 执行主函数
main
~                                                                                                                                                                                                                                            ~       