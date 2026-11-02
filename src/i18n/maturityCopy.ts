import type { Language } from "./types";

export type MaturityCopy = {
  startupLoadingTitle: string;
  startupLoadingDescription: string;
  databaseDescription: string;
  retry: string;
  troubleshooting: string;
  troubleshootingDescription: string;
  technicalDetails: string;
  viewErrorTitle: string;
  viewErrorDescription: string;
  backToOverview: string;
  openSettings: string;
};

const zh: MaturityCopy = {
  startupLoadingTitle: "正在准备 Fylura",
  startupLoadingDescription: "正在打开本地数据与文件空间。",
  databaseDescription: "Fylura 暂时无法打开本地数据。你可以重试；如果问题持续，请展开故障排查信息。",
  retry: "重试",
  troubleshooting: "故障排查",
  troubleshootingDescription: "先重试一次。如果仍然失败，请重新启动 Fylura，并确认应用的本地数据位置仍可访问。",
  technicalDetails: "显示技术详情",
  viewErrorTitle: "此页面暂时无法显示",
  viewErrorDescription: "页面遇到问题，但其他本地数据与功能仍保持原状。请重试，或切换到其他稳定页面继续使用。",
  backToOverview: "返回概览",
  openSettings: "打开设置"
};

const en: MaturityCopy = {
  startupLoadingTitle: "Preparing Fylura",
  startupLoadingDescription: "Opening your local data and file workspace.",
  databaseDescription: "Fylura cannot open its local data right now. Retry, or open troubleshooting details if the problem continues.",
  retry: "Retry",
  troubleshooting: "Troubleshooting",
  troubleshootingDescription: "Retry once first. If it still fails, restart Fylura and confirm that the app's local data location is still accessible.",
  technicalDetails: "Show technical details",
  viewErrorTitle: "This page cannot be shown right now",
  viewErrorDescription: "This page hit a problem, but your other local data and features remain unchanged. Retry, or switch to another stable page to keep working.",
  backToOverview: "Back to Overview",
  openSettings: "Open Settings"
};

export function maturityCopy(language: Language): MaturityCopy {
  return language === "en" ? en : zh;
}
