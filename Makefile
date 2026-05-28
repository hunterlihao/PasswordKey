APP_NAME = PasswordManager
VERSION = 1.0.0

.PHONY: build dev installer release clean

# 开发模式
dev:
	wails dev

# 构建
build:
	wails build

# 生成安装包
installer: build
	makensis build/windows/installer/project.nsi

# 完整流程
release: clean build installer

clean:
	rm -rf build/bin/*.exe
