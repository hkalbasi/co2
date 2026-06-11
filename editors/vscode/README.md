# CO2 Language Support for VS Code

Syntax highlighting for [CO2](https://github.com/hkalbasi/co2) programming language.

## Installation

Install from the VS Code Marketplace, or build from source:

```bash
cd editors/vscode
vsce package
code --install-extension co2-language-0.1.0.vsix
```

## Grammar

The CO2 TextMate grammar is based on the [better-c-syntax](https://github.com/jeff-hykin/better-c-syntax) grammar,
extended with CO2-specific patterns.
