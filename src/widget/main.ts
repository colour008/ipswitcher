import { createApp } from "vue";
import {
	ElButton,
	ElIcon,
	ElSlider,
	ElTag,
	ElTooltip,
} from "element-plus";
import "element-plus/es/components/button/style/css";
import "element-plus/es/components/message/style/css";
import "element-plus/es/components/slider/style/css";
import "element-plus/es/components/tag/style/css";
import "element-plus/es/components/tooltip/style/css";
import "element-plus/theme-chalk/dark/css-vars.css";
import Widget from "./Widget.vue";
import "@/styles/global.css";

// 透明挂件：去掉 body 默认底色与默认边距，只保留挂件卡片自身背景
document.body.style.background = "transparent";
const reset = document.createElement("style");
reset.textContent =
	"html,body,#app{margin:0;padding:0;width:100%;height:100%;background:transparent!important;border:none!important;outline:none!important;box-shadow:none!important;overflow:hidden;}";
document.head.appendChild(reset);

const app = createApp(Widget);

// 按需注册挂件实际使用的 Element Plus 组件
[ElButton, ElIcon, ElSlider, ElTag, ElTooltip].forEach(
	(comp) => app.use(comp)
);

app.mount("#app");

// 挂件主题跟随全局配置（缺省深色），具体明暗由 Widget.vue 读取配置后应用
