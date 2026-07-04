#pragma once
// code_mate — 输入:板载 BOOT 按钮(GPIO0)去抖 + 短按/长按区分(阶段7 起;阶段11 #1 加长按)。
//   BTN_DOWN  按下瞬间(实例切换的本地黑遮罩即时反馈)
//   BTN_SHORT 短按弹起(按住 < 长按阈值):实例切换 / 模式菜单里切换选中项
//   BTN_LONG  按住跨过长按阈值(按住中触发一次):进入 / 应用模式选择菜单
enum BtnEvent { BTN_NONE, BTN_DOWN, BTN_SHORT, BTN_LONG };

void input_begin();      // ⚠️ 必须在 setup()(boot 之后)调用:配置 GPIO0 为上拉输入
BtnEvent input_poll();   // 去抖后每拍返回一个事件;无变化返回 BTN_NONE
