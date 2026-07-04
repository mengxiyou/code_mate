#pragma once
// code_mate — system 布局(阶段12):本机 CPU / 内存 / 显存 仪表盘。
//   无 CC 会话时默认显示(取代旧 loading「No Session Available」);也作常驻伪实例在 BOOT 循环里。
//   LED 由磁盘活动驱动(忙=暖琥珀闪 / 闲=柔和蓝);磁盘只驱动灯,屏上不显示。
#include <lvgl.h>
#include "layout.h"

Layout *ui_system_layout();   // 返回该布局注册项(供 main 注册表)
