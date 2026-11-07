complete -c irsty -s c -l config -d 'Configuration file [default: config.toml in the platform config directory]' -r -F
complete -c irsty -l completions -d 'Print completions for SHELL, and exit' -r -f -a "bash\t''
elvish\t''
fish\t''
powershell\t''
zsh\t''"
complete -c irsty -l headless -d 'Run without the terminal UI, printing every buffer\'s new lines to stdout'
complete -c irsty -l plain -d 'A line-by-line interface for screen readers and braille displays: nothing is drawn, lines are printed as they arrive, and commands are read a line at a time'
complete -c irsty -l check-config -d 'Check the configuration file, report any problems, and exit (status 1 if there are any)'
complete -c irsty -l print-default-config -d 'Print a configuration file with every setting at its default, and exit'
complete -c irsty -l man -d 'Print the man page (roff), and exit'
complete -c irsty -s h -l help -d 'Print help (see more with \'--help\')'
complete -c irsty -s V -l version -d 'Print version'
