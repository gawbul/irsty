
using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'irsty' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $commandElements = $commandAst.CommandElements
    $command = @(
        'irsty'
        for ($i = 1; $i -lt $commandElements.Count; $i++) {
            $element = $commandElements[$i]
            if ($element -isnot [StringConstantExpressionAst] -or
                $element.StringConstantType -ne [StringConstantType]::BareWord -or
                $element.Value.StartsWith('-') -or
                $element.Value -eq $wordToComplete) {
                break
        }
        $element.Value
    }) -join ';'

    $completions = @(switch ($command) {
        'irsty' {
            [CompletionResult]::new('-c', '-c', [CompletionResultType]::ParameterName, 'Configuration file [default: config.toml in the platform config directory]')
            [CompletionResult]::new('--config', '--config', [CompletionResultType]::ParameterName, 'Configuration file [default: config.toml in the platform config directory]')
            [CompletionResult]::new('--completions', '--completions', [CompletionResultType]::ParameterName, 'Print completions for SHELL, and exit')
            [CompletionResult]::new('--headless', '--headless', [CompletionResultType]::ParameterName, 'Run without the terminal UI, printing every buffer''s new lines to stdout')
            [CompletionResult]::new('--plain', '--plain', [CompletionResultType]::ParameterName, 'A line-by-line interface for screen readers and braille displays: nothing is drawn, lines are printed as they arrive, and commands are read a line at a time')
            [CompletionResult]::new('--check-config', '--check-config', [CompletionResultType]::ParameterName, 'Check the configuration file, report any problems, and exit (status 1 if there are any)')
            [CompletionResult]::new('--print-default-config', '--print-default-config', [CompletionResultType]::ParameterName, 'Print a configuration file with every setting at its default, and exit')
            [CompletionResult]::new('--man', '--man', [CompletionResultType]::ParameterName, 'Print the man page (roff), and exit')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('-V', '-V ', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('--version', '--version', [CompletionResultType]::ParameterName, 'Print version')
            break
        }
    })

    $completions.Where{ $_.CompletionText -like "$wordToComplete*" } |
        Sort-Object -Property ListItemText
}
