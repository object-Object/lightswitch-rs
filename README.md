# lightswitch-rs

This is the API I wrote to control my light switch from my iPhone, using a Raspberry Pi, a servo, and a 3D printed bracket.

## Demo

![Loop of the device flipping the light on and off](flip.gif)

## Brackets

* [Toggle/flip switch](https://cad.onshape.com/documents/8eab044576e49e483864e1f3/w/e6adcae5591acaec7c86fcd0/e/8c350fa1b01b175b739c588d?renderMode=0&uiState=6460784a30af470c88b68eaf)
* [Rocker switch](https://www.thingiverse.com/thing:5217857)

## Installation

1. 3D print one of the above brackets, install an MG90S servo and a Raspberry Pi on it, and mount it to your light switch.
2. Install the correct toolchain for cross compiling to your Raspberry Pi.
3. Clone the repo. If necessary, change the constants in the `servo` module to match the datasheet of the servo you're using.
4. Build the crate, then transfer the executable and the `Rocket.toml` file to your Pi. [Enable hardware PWM](https://docs.golemparts.com/rppal/0.13.1/rppal/pwm/index.html#pwm-channels) on your Pi.
5. Run the executable to generate an API key, found in the `config.toml` file. You can keep it running however you wish (I use systemd).

## Usage

You can control the light switch by making requests to endpoints at the Raspberry Pi's ip address. See `main.rs` for all the endpoints. Here are some iOS Shortcuts you can use to control it (make sure to modify them with your Pi's ip address and generated API key):

- [Light On](https://www.icloud.com/shortcuts/3c690bb28fc44f1b8a8d1f32cb1aeac5): Turns the light on. Change the `state` field to `Off` to make it turn the light off instead.
- [Get Wake Up Time](https://www.icloud.com/shortcuts/f3875a054ecf45fdb307d659aa3334d8): Technically this isn't specific to this project. This returns the date/time of your Sleep Schedule alarm. The input should be on the day of the alarm you want to get, sometime between midnight and the time of the alarm (I use 1 AM).
- [Schedule Light On](https://www.icloud.com/shortcuts/9aa53c051f94471f890f5a7165f5bb33): Schedules the switch to turn on at the inputted time. I use this with the previous shortcut to turn on my light 15 minutes before my alarm goes off.

## Todo

- Bug: servo sometimes twitches slightly, not sure if I can stop that
- Create frontend
