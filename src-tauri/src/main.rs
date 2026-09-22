// Windows：显式声明 GUI 子系统，避免双击 exe 弹出 console 宿主窗口。
// 这个 #[windows_subsystem] 属性只在 Windows target 上生效，其他平台无副作用。
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    llens_lib::run()
}
