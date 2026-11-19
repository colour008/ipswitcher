import { createApp } from "vue";
import {
	ElButton,
	ElDescriptions,
	ElDescriptionsItem,
	ElDialog,
	ElForm,
	ElFormItem,
	ElIcon,
	ElInput,
	ElOption,
	ElRadio,
	ElRadioButton,
	ElRadioGroup,
	ElSelect,
	ElSwitch,
	ElTabPane,
	ElTabs,
	ElTag,
	ElTooltip,
} from "element-plus";
import "element-plus/dist/index.css";
import "element-plus/theme-chalk/dark/css-vars.css";
import App from "./App.vue";
import "./styles/global.css";

const app = createApp(App);

// 按需注册主窗实际使用的 Element Plus 组件（图标已在各组件内显式引入，不再全局全量注册）
[
	ElButton,
	ElDescriptions,
	ElDescriptionsItem,
	ElDialog,
	ElForm,
	ElFormItem,
	ElIcon,
	ElInput,
	ElOption,
	ElRadio,
	ElRadioButton,
	ElRadioGroup,
	ElSelect,
	ElSwitch,
	ElTabPane,
	ElTabs,
	ElTag,
	ElTooltip,
].forEach((comp) => app.use(comp));

app.mount("#app");

// 启动默认深色；具体主题由 App.vue 读取配置后校正，不再跟随系统明暗
document.documentElement.classList.add("dark");
