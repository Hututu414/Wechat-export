# 应用图标

`icon-source.png`：通过内置 imagegen 工具生成的项目原始图稿，保留透明通道。`app.ico`：16/24/32/48/64/128/256 像素 Windows 图标；`app.png`：128 像素窗口图标。后两者只做尺寸和格式转换，未改变设计。

图标已通过 `assets/app.rc` 嵌入 EXE，并由 egui 窗口加载。不复制微信官方图标。

生成提示词：

> Use case: logo-brand. Create ONE polished Windows desktop application icon for a lightweight private local chat-history export utility. Square 1024 x 1024 PNG with a genuinely transparent background outside the icon. A distinctive rounded-square deep emerald tile with softly rounded corners and subtle premium depth, centered white chat bubble integrated with a bold downward export arrow landing into a small archive tray, simple geometric silhouette readable at 16 px and 32 px. Restrained emerald, mint and warm white palette; crisp generous negative space, careful optical balance, clean modern native desktop feel, very subtle lighting, no busy gradients. The tile should occupy about 88 percent of the canvas with consistent transparent padding. No words, letters, numbers, watermark, mockup, surrounding objects, extra icon variants or official WeChat logo. Deliver the finished icon artwork, not a presentation.

工具实际返回 1254 × 1254 图稿，已核验 alpha 范围 0–255；ICO 七档尺寸均可解码。
