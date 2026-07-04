/* code_mate — LVGL v9.5 配置(部分覆盖;未定义的宏由 lv_conf_internal.h 取默认值) */
#ifndef LV_CONF_H
#define LV_CONF_H

/* 注意:本文件会被 LVGL 的架构汇编文件(如 lv_blend_helium.S)间接 include,
   必须保持「汇编安全」——只放预处理宏,切勿 #include <stdint.h> 等 C 头文件。 */

/* ---- 颜色 ---- */
#define LV_COLOR_DEPTH 16
/* v9 无 LV_COLOR_16_SWAP;字节序在 flush 时由 TFT_eSPI setSwapBytes 处理 */

/* ---- 内存(内置分配器)---- */
#define LV_USE_STDLIB_MALLOC   LV_STDLIB_BUILTIN
#define LV_USE_STDLIB_STRING   LV_STDLIB_BUILTIN
#define LV_USE_STDLIB_SPRINTF  LV_STDLIB_BUILTIN
#define LV_MEM_SIZE (64 * 1024U)   /* 阶段6:文本屏 spangroup + 两屏控件共用此堆,48→64K 留余量(OOM=assert 硬死机) */

/* ---- OS / tick ---- */
#define LV_USE_OS LV_OS_NONE
/* tick 在运行时用 lv_tick_set_cb(millis) 提供 */

/* ---- 渲染 ---- */
#define LV_DEF_REFR_PERIOD 33
#define LV_USE_DRAW_SW 1

/* ---- 日志 ---- */
#define LV_USE_LOG 0

/* ---- 控件(默认即 1,显式声明保险)---- */
#define LV_USE_ARC    1
#define LV_USE_BAR    1
#define LV_USE_LABEL  1
#define LV_USE_LINE   1
#define LV_USE_CANVAS 1
#define LV_USE_SPAN   1   /* 富文本:文本滚动屏按 markdown 分色(lv_spangroup,阶段6)*/

/* ---- 字体 ---- */
#define LV_FONT_FMT_TXT_LARGE 1   /* CJK 全字符集:字形多/位图 >1MB,需 32 位索引(否则报 Too large font)*/
#define LV_USE_FONT_COMPRESSED 1  /* font_cjk24 是压缩字形(bitmap_format=1,生成时未 --no-compress);不开则解不开→全豆腐块 */
#define LV_FONT_MONTSERRAT_12 1
#define LV_FONT_MONTSERRAT_14 1
#define LV_FONT_MONTSERRAT_20 1
#define LV_FONT_MONTSERRAT_28 1
#define LV_FONT_MONTSERRAT_36 1
#define LV_FONT_DEFAULT &lv_font_montserrat_14

#endif /* LV_CONF_H */
