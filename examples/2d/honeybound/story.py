"""Honeybound's story: a hero's journey told in a few lines at a time, never with choices.

The call: Baron Thornwick has stolen the Heartcomb, whose warmth keeps the Great Hive alive through winter, broken it into eight shards and grown thorns
over the sky so no bee can fly out. Pip, the smallest bee, cannot fly yet, so Pip is the only one who can go: on foot, before the first snow.
The road: a mentor (old Bramwell), bugs along the way who tell Pip how to beat what is ahead, seven of the Baron's lieutenants each holding a shard and
a prisoner from the hive, and the Baron himself. The return: the Heartcomb whole, the hive warm, and Pip's wings grown on the way.
"""

# ---- the call ---------------------------------------------------------------------------------------------------------------------------------------


def prologue():
    return [
        {"text": "THE GREAT HIVE, ON THE LAST WARM NIGHT OF AUTUMN."},
        {"who": "QUEEN MELLI", "text": "PIP! THE HEARTCOMB IS GONE. BARON THORNWICK BROKE IT INTO EIGHT SHARDS AND GAVE THEM TO HIS LIEUTENANTS."},
        {"who": "QUEEN MELLI", "text": "WITHOUT ITS WARMTH THE HIVE WILL FREEZE WITH THE FIRST SNOW. AND HIS THORNS NOW COVER THE SKY: NOT ONE BEE CAN FLY OUT."},
        {"who": "PIP", "text": "BUT... I CAN'T EVEN FLY YET."},
        {"who": "QUEEN MELLI", "text": "THE THORNS STOP WINGS, LITTLE ONE. NOT FEET. YOU ARE THE ONLY ONE OF US WHO CAN WALK OUT OF HERE."},
        {"who": "QUEEN MELLI", "text": "BRING THE EIGHT SHARDS HOME BEFORE THE SNOW. AND BRING BACK OUR FRIENDS HE TOOK. THE WHOLE HIVE IS COUNTING ON YOU."},
    ]


# ---- the road --------------------------------------------------------------------------------------------------------------------------------------
# stage index -> (who, line). Each world's first bug tells Pip how to beat its boss.
FOLK = {
    0: ("OLD BRAMWELL", "HOLD JUMP AS YOU FALL TO FLUTTER, PIP. SMALL WINGS STILL COUNT!"),
    2: ("DOT THE LADYBUG", "GRUBMAW KEEPS THE FIRST SHARD IN THORNKEEP. HE CAN'T SEE ABOVE HIS OWN HEAD: LAND ON IT!"),
    4: ("FENNICK THE CRICKET", "MADAM DUSKWING STOLE EVERY LANTERN IN THE WOOD. AFTER SHE SWOOPS SHE MUST REST ON THE GROUND."),
    8: ("INCHLEY", "CAPTAIN BURR ROLLS UP INTO A BALL OF PRICKLES. JUMP HIM, THEN BOP HIM WHILE HE'S DIZZY."),
    12: ("LUMI THE FIREFLY", "RIMEWEAVER DROPS FROM THE ROOF ON HER THREAD. THE FALL DAZES HER: JUMP ON HER BACK!"),
    16: ("DOT THE LADYBUG", "IF THE SAND STOPS AND SHIVERS, BACK AWAY. SANDMAW BURSTS OUT, AND HIS HEAD IS YOURS."),
    20: ("INCHLEY", "GLOAM TOOK THE FIREFLIES' LIGHT. AFTER HE THROWS SPARKS HE GETS SLEEPY AND SINKS DOWN."),
    24: ("FENNICK THE CRICKET", "QUEEN VESPA DIVES STINGER FIRST. WHEN SHE HUMS AND STOPS, MOVE! IF SHE MISSES SHE GETS STUCK."),
    28: ("LUMI THE FIREFLY", "THE BARON CHARGES LIKE A RUNAWAY ACORN. LET HIM HIT THE WALL, THEN JUMP ON HIS BACK."),
}

BOSS_NAMES = ["GRUBMAW", "MADAM DUSKWING", "CAPTAIN BURR", "RIMEWEAVER", "SANDMAW", "GLOAM", "QUEEN VESPA", "BARON THORNWICK"]
BOSS_TITLES = ["THE LARDER GRUB", "WHO HOARDS THE LIGHT", "THE BRAMBLE WEEVIL", "THE FROST SPIDER", "THE ANTLION OF THE DUNES", "THE LANTERN THIEF",
               "OF THE THORN WASPS", "LORD OF THORNS"]

# the seven friends the lieutenants hold, and what each says when Pip sets them free: who they are, where the road goes, a send-off
RESCUE = [
    ("BUZZ: PIP? LITTLE PIP CAME FOR ME?", "THE BARON'S ARMY MARCHED EAST INTO THE DUSKWOOD. A MOTH THERE HAS THE NEXT SHARD.",
     "I'LL FLY HOME THE LOW WAY AND TELL THE QUEEN YOU'RE COMING. ONE SHARD DOWN!"),
    ("MARIGOLD: OH! THE LANTERNS ARE LIT AGAIN!", "MY MAPS SAY THE NEXT SHARD IS IN AUTUMN HOLLOW, WITH A WEEVIL WHO CALLS HIMSELF A CAPTAIN.",
     "THE LEAVES ARE FALLING, PIP. WINTER IS CLOSE. HURRY!"),
    ("HAZEL: THE CAPTAIN WRAPPED ME IN BRAMBLES FOR SINGING TOO LOUD.", "NORTH LIES FROSTVALE. THE SNOW HAS ALREADY STARTED THERE.",
     "FIVE SHARDS LEFT. YOU'RE GETTING BRAVER WITH EVERY STEP."),
    ("SORREL: BRRR! YOU WALKED ALL THE WAY THROUGH THE SNOW?", "THE BARON'S CARRIAGE WENT SOUTH, OVER THE SUNSET DUNES. HE IS AFRAID OF YOU NOW.",
     "AND PIP... YOUR WINGS LOOK BIGGER. JUST A LITTLE."),
    ("THISTLEDOWN: A WHOLE ROYAL GUARD, AND IT TOOK THE SMALLEST OF US TO GET ME OUT.", "BEYOND THE DUNES IS THE STARLIT MEADOW. SOMEONE THERE STOLE THE STARS' LIGHT.",
     "GO QUIETLY. GO QUICKLY."),
    ("FENNEL: THEY MADE ME BAKE THORN PIES ALL WEEK. NOBODY LIKED THEM.", "QUEEN VESPA HOLDS THE NEXT SHARD ON BLOSSOM RIDGE. SHE HAS AN ARMY OF WASPS.",
     "HERE: A LITTLE HONEY FOR THE ROAD. YOU'VE EARNED IT."),
    ("BURDOCK: SEVEN SHARDS! THE HIVE WILL SING ABOUT THIS FOR A HUNDRED SUMMERS.", "THE LAST ONE IS WITH THE BARON HIMSELF, IN HIS CITADEL IN THE ASHLANDS.",
     "FINISH THIS, PIP. THE WHOLE HIVE IS WITH YOU."),
]

# a line on the card of each world's first stage: where the road has brought Pip
WORLD_LINES = [
    "THE ROAD BEGINS. THE THORNS ARE ALREADY CREEPING INTO THE MEADOW.",
    "THE DUSKWOOD, WHERE THE LANTERNS HAVE GONE OUT.",
    "AUTUMN HOLLOW. THE LEAVES ARE FALLING FAST NOW.",
    "FROSTVALE. THE FIRST SNOW IS ONLY A FEW DAYS AWAY.",
    "THE SUNSET DUNES: WARM SAND, AND SOMETHING MOVING UNDER IT.",
    "THE STARLIT MEADOW, WHERE THE FIREFLIES HAVE NO LIGHT.",
    "BLOSSOM RIDGE. THE WASPS ARE WATCHING THE ROAD.",
    "THE ASHLANDS. THE BARON'S CITADEL STANDS AT THEIR HEART.",
]


def final():
    return [
        {"who": "BARON THORNWICK", "text": "BEATEN... BY A BEE WHO CANNOT EVEN FLY."},
        {"who": "PIP", "text": "I DIDN'T NEED TO FLY. I JUST NEEDED TO KEEP WALKING."},
        {"text": "THE EIGHTH SHARD SETTLES INTO PLACE. THE HEARTCOMB IS WHOLE, AND ALL OVER THE ASHLANDS THE THORNS BEGIN TO WITHER."},
        {"text": "NOW: HOME, BEFORE THE SNOW."},
    ]


def ending():
    return [
        {"who": "QUEEN MELLI", "text": "YOU DID IT, PIP. FEEL THAT? THE HIVE IS WARM AGAIN, AND THE SNOW CAN FALL ALL IT LIKES."},
        {"who": "QUEEN MELLI", "text": "AND LOOK AT YOUR WINGS. A JOURNEY LIKE THAT WOULD GROW ANYONE'S WINGS."},
        {"who": "PIP", "text": "I'M... FLYING. I'M FLYING!"},
    ]


def tables(stage_levels):
    n = len(stage_levels)
    return {
        "stage_names": [L.name for L in stage_levels],
        "folk_names": [FOLK.get(i, (" ", ""))[0] for i in range(n)],
        "folk_lines": [FOLK.get(i, ("", " "))[1] for i in range(n)],
        "boss_names": BOSS_NAMES,
        "boss_titles": BOSS_TITLES,
        "world_lines": WORLD_LINES,
        **{f"rescue_{k + 1}": [row[k] for row in RESCUE] for k in range(3)},
    }
