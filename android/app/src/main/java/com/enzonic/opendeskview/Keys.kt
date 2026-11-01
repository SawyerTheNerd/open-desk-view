package com.enzonic.opendeskview

import android.view.KeyEvent

/** Maps Android key codes to Windows virtual-key codes for the keyboard message. */
object Keys {
    private val map: Map<Int, Int> = buildMap {
        for (i in 0 until 26) put(KeyEvent.KEYCODE_A + i, 0x41 + i) // A..Z
        for (i in 0 until 10) put(KeyEvent.KEYCODE_0 + i, 0x30 + i) // 0..9
        for (i in 0 until 12) put(KeyEvent.KEYCODE_F1 + i, 0x70 + i) // F1..F12
        for (i in 0 until 10) put(KeyEvent.KEYCODE_NUMPAD_0 + i, 0x60 + i) // numpad 0..9

        put(KeyEvent.KEYCODE_ENTER, 0x0D)
        put(KeyEvent.KEYCODE_NUMPAD_ENTER, 0x0D)
        put(KeyEvent.KEYCODE_ESCAPE, 0x1B)
        put(KeyEvent.KEYCODE_DEL, 0x08) // Backspace
        put(KeyEvent.KEYCODE_FORWARD_DEL, 0x2E) // Delete
        put(KeyEvent.KEYCODE_TAB, 0x09)
        put(KeyEvent.KEYCODE_SPACE, 0x20)
        put(KeyEvent.KEYCODE_CAPS_LOCK, 0x14)

        put(KeyEvent.KEYCODE_SHIFT_LEFT, 0xA0)
        put(KeyEvent.KEYCODE_SHIFT_RIGHT, 0xA1)
        put(KeyEvent.KEYCODE_CTRL_LEFT, 0xA2)
        put(KeyEvent.KEYCODE_CTRL_RIGHT, 0xA3)
        put(KeyEvent.KEYCODE_ALT_LEFT, 0xA4)
        put(KeyEvent.KEYCODE_ALT_RIGHT, 0xA5)
        put(KeyEvent.KEYCODE_META_LEFT, 0x5B) // Chromebook Search/Launcher -> Windows key
        put(KeyEvent.KEYCODE_META_RIGHT, 0x5C)

        put(KeyEvent.KEYCODE_DPAD_LEFT, 0x25)
        put(KeyEvent.KEYCODE_DPAD_UP, 0x26)
        put(KeyEvent.KEYCODE_DPAD_RIGHT, 0x27)
        put(KeyEvent.KEYCODE_DPAD_DOWN, 0x28)
        put(KeyEvent.KEYCODE_MOVE_HOME, 0x24)
        put(KeyEvent.KEYCODE_MOVE_END, 0x23)
        put(KeyEvent.KEYCODE_PAGE_UP, 0x21)
        put(KeyEvent.KEYCODE_PAGE_DOWN, 0x22)
        put(KeyEvent.KEYCODE_INSERT, 0x2D)

        put(KeyEvent.KEYCODE_MINUS, 0xBD)
        put(KeyEvent.KEYCODE_EQUALS, 0xBB)
        put(KeyEvent.KEYCODE_LEFT_BRACKET, 0xDB)
        put(KeyEvent.KEYCODE_RIGHT_BRACKET, 0xDD)
        put(KeyEvent.KEYCODE_BACKSLASH, 0xDC)
        put(KeyEvent.KEYCODE_SEMICOLON, 0xBA)
        put(KeyEvent.KEYCODE_APOSTROPHE, 0xDE)
        put(KeyEvent.KEYCODE_COMMA, 0xBC)
        put(KeyEvent.KEYCODE_PERIOD, 0xBE)
        put(KeyEvent.KEYCODE_SLASH, 0xBF)
        put(KeyEvent.KEYCODE_GRAVE, 0xC0)

        put(KeyEvent.KEYCODE_NUMPAD_ADD, 0x6B)
        put(KeyEvent.KEYCODE_NUMPAD_SUBTRACT, 0x6D)
        put(KeyEvent.KEYCODE_NUMPAD_MULTIPLY, 0x6A)
        put(KeyEvent.KEYCODE_NUMPAD_DIVIDE, 0x6F)
        put(KeyEvent.KEYCODE_NUMPAD_DOT, 0x6E)
    }

    // Keys Windows expects with KEYEVENTF_EXTENDEDKEY (navigation cluster, right-hand modifiers,
    // keypad Enter and /). Keyed by Android code: keypad Enter shares Enter's virtual-key code.
    private val extended = setOf(
        KeyEvent.KEYCODE_PAGE_UP, KeyEvent.KEYCODE_PAGE_DOWN, KeyEvent.KEYCODE_MOVE_END,
        KeyEvent.KEYCODE_MOVE_HOME, KeyEvent.KEYCODE_DPAD_LEFT, KeyEvent.KEYCODE_DPAD_UP,
        KeyEvent.KEYCODE_DPAD_RIGHT, KeyEvent.KEYCODE_DPAD_DOWN, KeyEvent.KEYCODE_INSERT,
        KeyEvent.KEYCODE_FORWARD_DEL, KeyEvent.KEYCODE_META_LEFT, KeyEvent.KEYCODE_META_RIGHT,
        KeyEvent.KEYCODE_CTRL_RIGHT, KeyEvent.KEYCODE_ALT_RIGHT, KeyEvent.KEYCODE_NUMPAD_DIVIDE,
        KeyEvent.KEYCODE_NUMPAD_ENTER,
    )

    /** Windows virtual-key code for [keyCode], or null if we don't forward that key. */
    fun toVk(keyCode: Int): Int? = map[keyCode]

    fun isExtended(keyCode: Int): Boolean = keyCode in extended
}
