package com.enzonic.opendeskview

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.view.Gravity
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast

/** Simple launcher: enter the server IP (or discover it on the LAN) and connect. */
class MainActivity : Activity() {

    private lateinit var ipField: EditText
    private lateinit var status: TextView

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
            hint = "server IP (e.g. 192.168.1.103)"
            setText("192.168.1.103")
            inputType = InputType.TYPE_CLASS_TEXT
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT
            )
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
            text = "Enter your PC's IP, or tap Discover. The spacedesk Server must be running."
            setPadding(0, pad, 0, 0)
        }

        root.addView(title)
        root.addView(ipField)
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
        startActivity(
            Intent(this, ViewerActivity::class.java)
                .putExtra(ViewerActivity.EXTRA_HOST, host)
                .putExtra(ViewerActivity.EXTRA_PORT, Protocol.DEFAULT_PORT)
        )
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
