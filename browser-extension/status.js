export function describeStatus(state) {
  const counts = state.hostCount > 0
    ? `${state.hostCount} 条整站规则、${state.urlCount} 条精确网址规则`
    : `${state.ruleCount} 条完整网址规则`;
  if (state.connection === "connected") {
    return {
      title: state.active ? "已连接 · 学习日屏蔽中" : "已连接 · 当前未屏蔽",
      detail: (state.active ? `已应用 ${counts}。暂停和休息时继续生效。` : "尚未开始学习日，或今天已经收工。")
        + (state.supportsHosts === false ? "主程序版本较旧，整站规则尚未同步到扩展，请更新主程序。" : ""),
    };
  }
  const reason = state.error === "response-error" ? "主程序返回的规则无效。"
    : state.error === "apply-error" ? "规则应用未完成，请重试同步。"
      : "暂时无法连接主程序。";
  return {
    title: state.connection === "cache" ? "离线 · 使用上次规则" : "等待连接坐功",
    detail: state.connection === "cache"
      ? `${reason}${state.active ? state.hostCount > 0 ? `继续沿用 ${counts}；收工后请打开主程序并同步。` : `继续屏蔽 ${state.ruleCount} 个网址；收工后请打开主程序并同步。` : "上次同步时未在学习日，当前没有生效规则。"}`
      : `${reason}请启动本机的坐功主程序，然后重试。首次成功同步前无法屏蔽。`,
  };
}
