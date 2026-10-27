package com.enzonic.opendeskview

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.view.Gravity
import android.view.ViewGroup
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.TextView
import android.widget.Toast

/**
 * Simple launcher: enter the server IP (or discover it on the LAN), pick how much CPU, bandwidth
 * and battery to spend, and connect. Choices are remembered between launches.
 */
class MainActivity : Activity() {

    private lateinit var ipField: EditText
    private lateinit var status: TextView
    private lateinit var h264Box: CheckBox
    private lateinit var perfGroup: RadioGroup

    private val prefs by lazy { getSharedPreferences("settings", MODE_PRIVATE) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val pad = (resources.displayMetrics.density * 24).toInt()
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad, pad, pad)
            gravity = Gravity.CENTER_HORIZONTAL
        }

        val title = TextView(this).apply {
            text = "OpenDeskView"
            textSize = 24f
            setPadding(0, 0, 0, pad)
        }

        ipField = EditText(this).apply {
            hint = "PC's IP address (e.g. 192.168.1.50)"
            setText(prefs.getString(PREF_HOST, ""))
            inputType = InputType.TYPE_CLASS_TEXT
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT
            )
        }

        h264Box = CheckBox(this).apply {
            text = "Use H.264 (hardware decode — lower bandwidth)"
            isChecked = prefs.getBoolean(PREF_H264, true)
        }

        val perfLabel = TextView(this).apply {
            text = "Performance"
            setPadding(0, pad / 2, 0, 0)
        }
        val saved = prefs.getString(PREF_PERFORMANCE, Performance.BALANCED.name)
        perfGroup = RadioGroup(this).apply {
            for (p in Performance.entries) {
                addView(RadioButton(this@MainActivity).apply {
                    id = p.ordinal + 1
                    text = p.label
                    isChecked = p.name == saved
                })
            }
        }

        val connectBtn = Button(this).apply {
            text = "Connect"
            setOnClickListener { connect() }
        }

        val discoverBtn = Button(this).apply {
            text = "Discover on LAN"
            setOnClickListener { discover() }
        }

        status = TextView(this).apply {
            text = "Enter your PC's IP, or tap Discover. The OpenDeskView server (or spacedesk) must be running on the PC."
            setPadding(0, pad, 0, 0)
        }

        root.addView(title)
        root.addView(ipField)
        root.addView(h264Box)
        root.addView(perfLabel)
        root.addView(perfGroup)
        root.addView(connectBtn)
        root.addView(discoverBtn)
        root.addView(status)
        setContentView(root)
    }

    private fun connect() {
        val host = ipField.text.toString().trim()
        if (host.isEmpty()) {
            Toast.makeText(this, "enter an IP first", Toast.LENGTH_SHORT).show()
            return
        }
        val perf = Performance.entries.getOrElse(perfGroup.checkedRadioButtonId - 1) { Performance.BALANCED }
        prefs.edit()
            .putString(PREF_HOST, host)
            .putBoolean(PREF_H264, h264Box.isChecked)
            .putString(PREF_PERFORMANCE, perf.name)
            .apply()
        startActivity(
            Intent(this, ViewerActivity::class.java)
                .putExtra(ViewerActivity.EXTRA_HOST, host)
                .putExtra(ViewerActivity.EXTRA_PORT, Protocol.DEFAULT_PORT)
                .putExtra(ViewerActivity.EXTRA_H264, h264Box.isChecked)
                .putExtra(ViewerActivity.EXTRA_FPS, perf.fps)
                .putExtra(ViewerActivity.EXTRA_QUALITY, perf.quality)
        )
    }

    companion object {
        private const val PREF_HOST = "host"
        private const val PREF_H264 = "h264"
        private const val PREF_PERFORMANCE = "performance"
    }

    private fun discover() {
        status.text = "discovering…"
        Thread {
            val servers = try {
                Discovery.discover(1500)
            } catch (e: Exception) {
                emptyList()
            }
            runOnUiThread {
                if (servers.isEmpty()) {
                    status.text = "no servers found. Enter the IP manually."
                } else {
                    ipField.setText(servers.first().ip)
                    status.text = "found:\n" + servers.joinToString("\n") { "• ${it.ip}  ${it.name}" }
                }
            }
        }.start()
    }
}
