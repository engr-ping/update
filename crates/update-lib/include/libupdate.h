/*
 * libupdate.h — update C ABI 共享库头文件（与 libupdate 动态库配套）。
 *
 * 用法：
 *   #include "libupdate.h"
 *   char *json = update_check(config, NULL);   /* 返回 malloc 字符串 */
 *   if (*update_last_error() == '\0') {
 *       /* 成功：json 为协议 JSON，自行解析 */
 *   } else {
 *       /* 失败：json 为错误消息，可显示给用户 */ 
 *   }
 *   update_free(json);          /* 必须释放，且只能用 update_free */
 *
 * 约定：
 *   - 所有返回 char* 的 API 均由库内分配，调用方必须用 update_free 释放；
 *     空指针安全，释放任意一次即够。
 *   - 业务失败时 API 返回错误消息字符串（非 NULL），并设置 last_error；
 *     判断成败请用 update_last_error() 是否为空串，不要依赖返回指针。
 *   - 协议与 update 二进制一致（见 docs/integration.md §4）：
 *     check 成功输出 {schema,current_version,latest_version,update_available,release}；
 *     download 成功输出 {schema,version,file}；退出码 0 成功 / 2 配置错误 /
 *     3 源错误 / 4 下载错误（本库无退出码，成败全看 update_last_error）。
 *   - 所有函数线程安全。
 */

#ifndef LIBUPDATE_H
#define LIBUPDATE_H

#ifdef __cplusplus
extern "C" {
#endif

/* 检查是否有新版本。
 * config_path: 配置文件路径（必填）。
 * current_version: 当前版本号，可 NULL；非 NULL 时作为 --current-version 传入。
 * 成功返回 check 协议 JSON；失败返回错误消息并设置 last_error。 */
extern char *update_check(const char *config_path, const char *current_version);

/* 下载指定版本产物到本地。
 * config_path: 配置文件路径（必填）。
 * version: 版本号或 "latest"（必填）。
 * asset: 资产名，可 NULL。
 * out: 输出路径，可 NULL（默认当前目录）。
 * 成功返回 download 协议 JSON；失败返回错误消息并设置 last_error。 */
extern char *update_download(const char *config_path, const char *version,
                             const char *asset, const char *out);

/* 列出可用版本。
 * config_path: 配置文件路径（必填）。
 * limit: 最多返回几个版本；<= 0 时用库默认值。
 * 成功返回 list 协议 JSON；失败返回错误消息并设置 last_error。 */
extern char *update_list(const char *config_path, int limit);

/* 返回 update 库自身版本号（纯文本）。 */
extern char *update_version(void);

/* 后台自动更新（阻塞式循环）。宿主应在自己起的线程里调用。
 * config_path:    配置文件路径（必填）。
 * interval_secs:  两次检查间隔秒数（<=0 视为默认 1 天）。
 * out_dir:        下载目录，可 NULL（默认系统临时目录）。
 * watch_pid:      宿主 PID（0 表示不监测）；非 0 时宿主退出且有就绪更新则执行 on_update。
 * on_update:      宿主退出后执行的 shell 模板，可 NULL，支持 {file}/{version} 占位符。
 * 返回退出码：0 成功。 */
extern int update_autoupdate_run(const char *config_path, unsigned long long interval_secs,
                                 const char *out_dir, unsigned int watch_pid,
                                 const char *on_update);

/* 宿主在退出钩子里显式应用已下载的更新。
 * template: shell 模板（支持 {file}/{version}）。
 * file:     已下载文件路径。
 * version:  版本号。
 * 成功返回空串；失败返回错误消息并设置 last_error。 */
extern char *update_apply(const char *template, const char *file, const char *version);

/* 返回最近一次失败的错误消息；返回 "" 表示无错误。成功调用时清空。 */
extern char *update_last_error(void);

/* 释放上面各函数返回的字符串（空指针安全）。 */
extern void update_free(char *ptr);

#ifdef __cplusplus
}
#endif

#endif /* LIBUPDATE_H */