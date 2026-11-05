"""Create tiny random test weights; this is not a useful pretrained model."""
import sys
from pathlib import Path
import torch
from tokenizers import Tokenizer
from tokenizers.models import WordLevel
from tokenizers.pre_tokenizers import Whitespace
from transformers import GPT2Config, GPT2LMHeadModel, PreTrainedTokenizerFast

folder = Path(sys.argv[1])
folder.mkdir(parents=True, exist_ok=True)
vocab = {word: index for index, word in enumerate([
    "[PAD]", "[UNK]", "[BOS]", "[EOS]", "hello", "test", "local", "model",
    "check", "error", "the", "answer", "is", "ready", ".", "permissions",
])}
tokenizer = Tokenizer(WordLevel(vocab, unk_token="[UNK]"))
tokenizer.pre_tokenizer = Whitespace()
fast = PreTrainedTokenizerFast(tokenizer_object=tokenizer, unk_token="[UNK]", pad_token="[PAD]", bos_token="[BOS]", eos_token="[EOS]")
fast.save_pretrained(folder)
torch.manual_seed(7)
model = GPT2LMHeadModel(GPT2Config(vocab_size=len(vocab), n_layer=1, n_head=2, n_embd=16,
    n_positions=512, bos_token_id=2, eos_token_id=3, pad_token_id=0))
model.generation_config.suppress_tokens = [0, 1, 2, 3]
model.save_pretrained(folder, safe_serialization=True)
print("Synthetic safetensors fixture created")
