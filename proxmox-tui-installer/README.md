# proxmox-tui-installer

## Testing/debugging

### stderr redirection

If something needs to be printed for debugging (e.g. using `eprintln!()` or
`dbg!()`), output redirection can be used. Open a second terminal and get the
file name of the terminal using the `tty` command:
```sh
$ tty
/dev/pts/6
```

Now, simply run the installer using:
```sh
$ cargo run 2>/dev/pts/6
```

All stderr output will then show up in the other terminal.

### Specific terminal size

To test the installer with a specific output size, the `stty` command can be
used. For example, to set it to a standard 80x25 terminal:
```sh
$ stty columns 80 rows 25
```
