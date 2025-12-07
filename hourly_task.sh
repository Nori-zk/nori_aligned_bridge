#!/bin/bash

# 环境变量配置（关键！cron 默认无用户环境变量）
SHELL=/bin/bash
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

# 日志配置
LOG_FILE="/var/log/hourly_task.log"
TIMESTAMP=$(date "+%Y-%m-%d %H:%M:%S")

# 任务内容示例（根据需求修改）
main() {
    echo "===== 任务开始 [${TIMESTAMP}] =====" >> "$LOG_FILE"
    
    # 示例1：清理临时文件（保留最近24小时）
    find /tmp -type f -mtime +0 -delete 2>>"$LOG_FILE"
    
    # 示例2：系统状态监控
    free -m >> "$LOG_FILE"
    df -h >> "$LOG_FILE"
    
    # 示例3：自定义命令
    # /path/to/your/command >> "$LOG_FILE" 2>&1
    cd /home/ubuntu/nori_workspace/mina_bridge/
    make submit_devnet_state
    
    echo "===== 任务结束 [${TIMESTAMP}] =====" >> "$LOG_FILE"
}

# 执行主函数
main