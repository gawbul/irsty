
use builtin;
use str;

set edit:completion:arg-completer[irsty] = {|@words|
    fn spaces {|n|
        builtin:repeat $n ' ' | str:join ''
    }
    fn cand {|text desc|
        edit:complex-candidate $text &display=$text' '(spaces (- 14 (wcswidth $text)))$desc
    }
    var command = 'irsty'
    for word $words[1..-1] {
        if (str:has-prefix $word '-') {
            break
        }
        set command = $command';'$word
    }
    var completions = [
        &'irsty'= {
            cand -c 'Configuration file [default: config.toml in the platform config directory]'
            cand --config 'Configuration file [default: config.toml in the platform config directory]'
            cand --completions 'Print completions for SHELL, and exit'
            cand --headless 'Run without the terminal UI, printing every buffer''s new lines to stdout'
            cand --plain 'A line-by-line interface for screen readers and braille displays: nothing is drawn, lines are printed as they arrive, and commands are read a line at a time'
            cand --check-config 'Check the configuration file, report any problems, and exit (status 1 if there are any)'
            cand --print-default-config 'Print a configuration file with every setting at its default, and exit'
            cand --man 'Print the man page (roff), and exit'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
            cand -V 'Print version'
            cand --version 'Print version'
        }
    ]
    $completions[$command]
}
