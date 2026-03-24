# VHS demo tape for eli README
# Run: vhs demo.tape

Output site/assets/demo.gif
Output site/assets/demo.webm

Set Shell "bash"
Set FontSize 16
Set Width 900
Set Height 500
Set Theme "Catppuccin Mocha"
Set FontFamily "JetBrains Mono"
Set Padding 20
Set Framerate 30
Set TypingSpeed 50ms
Set CursorBlink false

Type "eli chat"
Enter
Sleep 1.5s

Type "> What can you do?"
Sleep 300ms
Enter
Sleep 3s

# Simulate streaming response
Type ""
Sleep 5s

Type "> summarize this repo in one paragraph"
Sleep 300ms
Enter
Sleep 3s

Type ""
Sleep 5s

# Exit
Type "/quit"
Enter
Sleep 1s
