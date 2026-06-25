> [!NOTE]
> **Supports Windows 10 and 11 only.** OCR requires language packs installed via **Windows Settings > Time & Language > Language & region**.

<h2 align="center">Description</h2>

QuietTools is a lightweight, native Windows utility that lives in your system tray. It provides quick-access color picking, OCR text extraction with global hotkeys and a keyboard-driven window manager like in KDE.

<h2 align="center">Features</h2>

**Color Picker** — `Ctrl` + `Shift` + `C`
- Click anywhere on screen to pick a color
- Copy HEX or RGB value to clipboard
- Pixel-level accuracy with magnified loupe
- Arrow keys for fine cursor movement (hold `Shift` for 5px steps)
<img width="533" height="299" alt="qt_cp" src="https://github.com/user-attachments/assets/c7cc86c3-b973-4ad4-81f3-93cff4141930" />

**Text Extractor** — `Ctrl` + `Alt` + `C`
- Drag to select a screen region
- Recognizes text offline via Windows OCR
- Supports English, Russian, Chinese, and any installed language pack
- Multi-language text in a single selection works


**Window Manager** — `Win` key + mouse
- `Win` + **Left-drag**: Move a window
- `Win` + `Shift` + **Left-drag**: Snap to zones (3×3 grid with halves)
- `Win` + **Right-drag**: Resize a window from nearest corner
- `Win` + **Middle-click**: Minimize a window
- `Win` + **Scroll**: Adjust window opacity
- `Win` + **Double-click**: Toggle maximize
<img width="533" height="301" alt="qt_wm" src="https://github.com/user-attachments/assets/707631dc-e1fd-433e-960f-10d762d01a8d" />

<h2 align="center">Installation</h2>

1. Download `QuietTools.exe` from the [Releases](../../releases) page.
2. Run the executable — it starts in the system tray.
3. *(Optional)* Enable **Run at startup** from the tray menu.

No installer, no dependencies, no administrator rights needed.

<h2 align="center">Build from source</h2>

1. Install [Rust](https://www.rust-lang.org/tools/install).
2. Clone the repository:
   ```
   git clone https://github.com/BEQI/QuietTools.git
   cd QuietTools
   ```
3. Build the release binary:
   ```
   cargo build --release
   ```
4. The executable is at `target/release/QuietTools.exe`.

<h2 align="center">Credits</h2>

<p align="center">
Developed by <a href="https://github.com/BEQI">BEQI</a>.
</p>

<h2 align="center">License</h2>

<p align="center">
<img src="https://www.gnu.org/graphics/gplv3-127x51.png" alt="GPLv3 License" />
</p>

<p align="center">
This project is licensed under the <strong>GPLv3</strong> License.
</p>
